use iri_string::{types::UriString, validate::Error as UriError};
use libipld::cid::{
    multihash::{Code, MultihashDigest},
    Cid,
};

use std::{convert::TryFrom, fmt, str::FromStr};
use thiserror::Error;

#[derive(Clone, Hash, PartialEq, Debug, Eq)]
pub struct OrbitId {
    suffix: String,
    id: String,
}

impl OrbitId {
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
            write!(f, "/{}", p)?
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
        match UriString::from_str(&s[p - 1..])
            .map(|uri| (uri, uri.as_slice().authority_components()))
            .map(|(u, a)| {
                (
                    &s[..p].strip_prefix("kepler:"),
                    a.host(),
                    a.port(),
                    a.userinfo(),
                    u.path_str(),
                    u.fragment(),
                    u.query_str(),
                )
            })? {
            (Some(suf), Some(a), None, None, None, None, None) => Ok(Self {
                suffix: suf.into(),
                id: a.host().into(),
            }),
            _ => Err(Self::Err::IncorrectForm),
        }
    }
}

impl FromStr for ResourceId {
    type Err = KRIParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (a, p, f) = s
            .split_once("://")
            .map(|(_, p)| (p.find(':'), p.find('/'), p.find('#')))
            .ok_or_else(|| Self::Err::IncorrectForm)?;
        let (orbit, service, path, fragment) = match (a, p, f) {
            (None, None, None) => (s.parse()?, None, None, None),
            (Some(a), None, None) => (s[..a].parse()?, Some(s[a..].into()), None, None),
            (Some(a), Some(p), Some(f)) if a < p && p < f => (
                s[..a].parse()?,
                Some(s[a..p].into()),
                Some(s[p..f].into()),
                Some(s[f..].into()),
            ),
            // (Some(a),)
        };
        Ok(Self {
            orbit,
            service,
            path,
            fragment,
        })
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
