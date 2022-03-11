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
        ["did", &self.suffix()].join(":")
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
    type Error = ();
    fn try_from(did: DIDURL) -> Result<Self, Self::Error> {
        match (did.did.strip_prefix("did:"), did.fragment) {
            (Some(s), Some(i)) => Ok(Self {
                suffix: s.into(),
                id: i.into(),
            }),
            _ => Err(()),
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
        let p = match s.find("://") {
            Some(p) if p > 0 => p,
            _ => Err(Self::Err::IncorrectForm)?,
        };
        let uri = UriString::from_str(&s[p - 1..])?;
        match uri.authority_components().map(|a| {
            (
                s[..p].strip_prefix("kepler:").map(|su| su.to_string()),
                a.host().to_string(),
                a.port(),
                a.userinfo(),
                uri.path_str(),
                uri.fragment(),
                uri.query_str(),
            )
        }) {
            Some((Some(suffix), id, None, None, "", None, None)) => Ok(Self { suffix, id }),
            _ => Err(Self::Err::IncorrectForm),
        }
    }
}

impl FromStr for ResourceId {
    type Err = KRIParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let p = match s.find("://") {
            Some(p) if p > 0 => p,
            _ => Err(Self::Err::IncorrectForm)?,
        };
        let uri = UriString::from_str(&s[p - 1..])?;
        match uri.authority_components().map(|a| {
            (
                s[..p].strip_prefix("kepler:").map(|su| su.to_string()),
                a.host(),
                a.port(),
                a.userinfo(),
            )
        }) {
            Some((Some(suffix), name, service, None)) => Ok(Self {
                orbit: OrbitId {
                    suffix,
                    id: name.into(),
                },
                service: service.map(|s| s.into()),
                path: match uri.path_str() {
                    "" => None,
                    ps => Some(ps.to_string()),
                },
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
    fn basic() {
        let uri = "kepler:ens:example.eth://orbit0:s3/path/to/image.jpg";
    }
}
