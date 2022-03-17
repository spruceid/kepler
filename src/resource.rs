use iri_string::{types::UriString, validate::Error as UriError};
use libipld::cid::{
    multihash::{Code, MultihashDigest},
    Cid,
};
use ssi::did::DIDURL;

use std::{convert::TryFrom, fmt, str::FromStr};
use thiserror::Error;

#[derive(Clone, Hash, PartialEq, Debug, Eq)]
pub struct OrbitId {
    suffix: String,
    id: String,
}

impl OrbitId {
    pub fn new(suffix: String, id: String) -> Self {
        Self { suffix, id }
    }

    pub fn did(&self) -> String {
        ["did", self.suffix()].join(":")
    }

    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    pub fn name(&self) -> &str {
        &self.id
    }

    pub fn get_cid(&self) -> Cid {
        Cid::new_v1(0x55, Code::Blake2b256.digest(self.to_string().as_bytes()))
    }
}

impl TryFrom<DIDURL> for OrbitId {
    type Error = KRIParseError;
    fn try_from(did: DIDURL) -> Result<Self, Self::Error> {
        match (
            did.did.strip_prefix("did:").map(|s| s.to_string()),
            did.fragment,
        ) {
            (Some(suffix), Some(id)) => Ok(Self { suffix, id }),
            _ => Err(KRIParseError::IncorrectForm),
        }
    }
}

#[derive(Clone, Hash, PartialEq, Debug)]
pub struct ResourceId {
    orbit: OrbitId,
    service: Option<String>,
    path: Option<String>,
    fragment: Option<String>,
}

impl ResourceId {
    pub fn orbit(&self) -> &OrbitId {
        &self.orbit
    }
    pub fn service(&self) -> &Option<String> {
        &self.service
    }
    pub fn path(&self) -> &Option<String> {
        &self.path
    }
    pub fn fragment(&self) -> &Option<String> {
        &self.fragment
    }
}

impl fmt::Display for OrbitId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "kepler:{}://{}", &self.suffix, &self.id)
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", &self.orbit)?;
        if let Some(s) = &self.service {
            write!(f, ":{}", s)?
        };
        if let Some(p) = &self.path {
            write!(f, "{}", p)?
        };
        if let Some(fr) = &self.fragment {
            write!(f, "#{}", fr)?
        };
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum KRIParseError {
    #[error("Incorrect Structure")]
    IncorrectForm,
    #[error(transparent)]
    InvalidUri(#[from] UriError),
}

impl FromStr for OrbitId {
    type Err = KRIParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s
            .strip_prefix("kepler:")
            .ok_or(KRIParseError::IncorrectForm)?;
        let p = match s.find("://") {
            Some(p) if p > 0 => p,
            _ => return Err(Self::Err::IncorrectForm),
        };
        let uri = UriString::from_str(&["dummy", &s[p..]].concat())?;
        match uri.authority_components().map(|a| {
            (
                a.host().to_string(),
                a.port(),
                a.userinfo(),
                uri.path_str(),
                uri.fragment(),
                uri.query_str(),
            )
        }) {
            Some((id, None, None, "", None, None)) => Ok(Self {
                suffix: s[..p].to_string(),
                id,
            }),
            _ => Err(Self::Err::IncorrectForm),
        }
    }
}

impl FromStr for ResourceId {
    type Err = KRIParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s
            .strip_prefix("kepler:")
            .ok_or(KRIParseError::IncorrectForm)?;
        let p = match s.find("://") {
            Some(p) if p > 0 => p,
            _ => return Err(Self::Err::IncorrectForm),
        };
        let uri = UriString::from_str(&["dummy", &s[p..]].concat())?;
        match uri.authority_components().map(|a| {
            (
                a.host(),
                a.userinfo(),
                uri.path_str().split_once('/').map(|(s, r)| match s {
                    "" => r.split_once('/').unwrap_or((r, "")),
                    _ => (s, r),
                }),
            )
        }) {
            Some((host, None, path)) => Ok(Self {
                orbit: OrbitId {
                    suffix: s[..p].to_string(),
                    id: host.into(),
                },
                service: path.map(|(s, _)| s.into()),
                path: path.map(|(_, pa)| ["/", pa].join("")),
                fragment: uri.fragment().map(|s| s.to_string()),
            }),
            _ => Err(Self::Err::IncorrectForm),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    async fn basic() {
        let res: ResourceId = "kepler:ens:example.eth://orbit0/s3/path/to/image.jpg"
            .parse()
            .unwrap();

        assert_eq!("ens:example.eth", res.orbit().suffix());
        assert_eq!("did:ens:example.eth", res.orbit().did());
        assert_eq!("orbit0", res.orbit().name());
        assert_eq!("s3", res.service().as_ref().unwrap());
        assert_eq!("/path/to/image.jpg", res.path().as_ref().unwrap());
        assert_eq!(None, res.fragment().as_ref());

        let res2: ResourceId = "kepler:ens:example.eth://orbit0#peer".parse().unwrap();

        assert_eq!("ens:example.eth", res2.orbit().suffix());
        assert_eq!("did:ens:example.eth", res2.orbit().did());
        assert_eq!("orbit0", res2.orbit().name());
        assert_eq!(None, res2.service().as_ref());
        assert_eq!(None, res2.path().as_ref());
        assert_eq!("peer", res2.fragment().as_ref().unwrap());
    }

    #[test]
    async fn failures() {
        let no_suffix: Result<ResourceId, _> = "kepler:://orbit0/s3/path/to/image.jpg".parse();
        assert!(no_suffix.is_err());

        let invalid_name: Result<ResourceId, _> =
            "kepler:ens:example.eth://or:bit0/s3/path/to/image.jpg".parse();
        assert!(invalid_name.is_err());
    }
}
