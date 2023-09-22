use iri_string::{
    types::{UriStr, UriString},
    validate::Error as UriError,
};
use libipld::{
    cbor::DagCborCodec,
    cid::{
        multihash::{Code, MultihashDigest},
        Cid,
    },
    codec::{Decode, Encode},
    error::Error as IpldError,
};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use ssi::did::DIDURL;

use std::io::{Read, Seek, Write};
use std::{convert::TryFrom, fmt, str::FromStr};
use thiserror::Error;

#[derive(
    Clone, Hash, PartialEq, Debug, Eq, SerializeDisplay, DeserializeFromStr, PartialOrd, Ord,
)]
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
        Cid::new_v1(
            0x55, // raw codec
            Code::Blake2b256.digest(self.to_string().as_bytes()),
        )
    }

    pub fn to_resource(
        self,
        service: Option<String>,
        path: Option<String>,
        fragment: Option<String>,
    ) -> ResourceId {
        ResourceId {
            orbit: self,
            service,
            path: path.map(|p| {
                if p.starts_with('/') {
                    p
                } else {
                    format!("/{p}")
                }
            }),
            fragment,
        }
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

#[derive(
    Clone, Hash, PartialEq, Debug, Eq, SerializeDisplay, DeserializeFromStr, PartialOrd, Ord,
)]
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
    pub fn service(&self) -> Option<&str> {
        self.service.as_ref().map(|s| s.as_ref())
    }
    pub fn path(&self) -> Option<&str> {
        self.path.as_ref().map(|s| s.as_ref())
    }
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_ref().map(|s| s.as_ref())
    }
    pub fn extends(&self, base: &ResourceId) -> Result<(), ResourceCheckError> {
        if base.orbit() != self.orbit() {
            Err(ResourceCheckError::IncorrectOrbit)
        } else if base.service() != self.service() {
            Err(ResourceCheckError::IncorrectService)
        } else if base.fragment() != self.fragment() {
            Err(ResourceCheckError::IncorrectFragment)
        } else if !self
            .path()
            .unwrap_or("")
            .starts_with(base.path().unwrap_or(""))
        {
            Err(ResourceCheckError::DoesNotExtendPath)
        } else {
            Ok(())
        }
    }

    pub fn into_inner(self) -> (OrbitId, Option<String>, Option<String>, Option<String>) {
        (self.orbit, self.service, self.path, self.fragment)
    }

    pub fn get_cid(&self) -> Cid {
        Cid::new_v1(
            0x55, // raw codec
            Code::Blake2b256.digest(self.to_string().as_bytes()),
        )
    }
}

#[derive(Error, Debug)]
pub enum ResourceCapErr {
    #[error("Missing ResourceId fragment")]
    MissingAction,
}

#[derive(Error, Debug)]
pub enum ResourceCheckError {
    #[error("Base and Extension Orbits do not match")]
    IncorrectOrbit,
    #[error("Base and Extension Services do not match")]
    IncorrectService,
    #[error("Base and Extension Fragments do not match")]
    IncorrectFragment,
    #[error("Extension does not extend path of Base")]
    DoesNotExtendPath,
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
            write!(f, "/{s}")?
        };
        if let Some(p) = &self.path {
            write!(f, "{p}")?
        };
        if let Some(fr) = &self.fragment {
            write!(f, "#{fr}")?
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
                path: path.map(|(_, pa)| format!("/{pa}")),
                fragment: uri.fragment().map(|s| s.to_string()),
            }),
            _ => Err(Self::Err::IncorrectForm),
        }
    }
}

impl TryFrom<UriString> for ResourceId {
    type Error = KRIParseError;
    fn try_from(u: UriString) -> Result<Self, Self::Error> {
        u.as_str().parse()
    }
}

impl<'a> TryFrom<&'a UriStr> for ResourceId {
    type Error = KRIParseError;
    fn try_from(u: &'a UriStr) -> Result<Self, Self::Error> {
        u.as_str().parse()
    }
}

impl TryFrom<&UriString> for ResourceId {
    type Error = KRIParseError;
    fn try_from(u: &UriString) -> Result<Self, Self::Error> {
        u.as_str().parse()
    }
}

impl Encode<DagCborCodec> for ResourceId {
    fn encode<W>(&self, c: DagCborCodec, w: &mut W) -> Result<(), IpldError>
    where
        W: Write,
    {
        self.to_string().encode(c, w)
    }
}

impl Decode<DagCborCodec> for ResourceId {
    fn decode<R>(c: DagCborCodec, r: &mut R) -> Result<Self, IpldError>
    where
        R: Read + Seek,
    {
        Ok(String::decode(c, r)?.parse()?)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[serde(untagged)]
pub enum AnyResource<O = UriString> {
    Kepler(ResourceId),
    Other(O),
}

impl<O> AnyResource<O> {
    pub fn orbit(&self) -> Option<&OrbitId> {
        match self {
            AnyResource::Kepler(id) => Some(id.orbit()),
            AnyResource::Other(_) => None,
        }
    }

    pub fn kepler_resource(&self) -> Option<&ResourceId> {
        match self {
            AnyResource::Kepler(id) => Some(id),
            AnyResource::Other(_) => None,
        }
    }
}

impl<O> AnyResource<O>
where
    O: AsRef<str>,
{
    pub fn extends<O2: AsRef<str>>(&self, other: &AnyResource<O2>) -> bool {
        match (self, other) {
            (AnyResource::Kepler(a), AnyResource::Kepler(b)) => a.extends(b).is_ok(),
            (AnyResource::Other(a), AnyResource::Other(b)) => a.as_ref().starts_with(b.as_ref()),
            _ => false,
        }
    }
}

impl<O> From<ResourceId> for AnyResource<O> {
    fn from(id: ResourceId) -> Self {
        AnyResource::Kepler(id)
    }
}

impl<'a> From<&'a UriStr> for AnyResource<&'a UriStr> {
    fn from(id: &'a UriStr) -> Self {
        id.as_str()
            .parse()
            .map(AnyResource::Kepler)
            .unwrap_or(AnyResource::Other(id))
    }
}

impl<'a> From<&'a UriString> for AnyResource<&'a UriStr> {
    fn from(id: &'a UriString) -> Self {
        id.as_str()
            .parse()
            .map(AnyResource::Kepler)
            .unwrap_or(AnyResource::Other(id))
    }
}

impl From<UriString> for AnyResource<UriString> {
    fn from(id: UriString) -> Self {
        id.as_str()
            .parse()
            .map(AnyResource::Kepler)
            .unwrap_or(AnyResource::Other(id))
    }
}

impl From<&UriString> for AnyResource<UriString> {
    fn from(id: &UriString) -> Self {
        id.as_str()
            .parse()
            .map(AnyResource::Kepler)
            .unwrap_or(AnyResource::Other(id.clone()))
    }
}

impl From<&UriStr> for AnyResource<UriString> {
    fn from(id: &UriStr) -> Self {
        id.as_str()
            .parse()
            .map(AnyResource::Kepler)
            .unwrap_or(AnyResource::Other(id.to_owned()))
    }
}

impl<'a> From<AnyResource<&'a UriString>> for AnyResource<&'a UriStr> {
    fn from(id: AnyResource<&'a UriString>) -> Self {
        match id {
            AnyResource::Kepler(id) => AnyResource::Kepler(id),
            AnyResource::Other(id) => AnyResource::Other(id.as_ref()),
        }
    }
}

impl<'a> From<AnyResource<&'a UriStr>> for AnyResource<UriString> {
    fn from(id: AnyResource<&'a UriStr>) -> Self {
        match id {
            AnyResource::Kepler(id) => AnyResource::Kepler(id),
            AnyResource::Other(id) => AnyResource::Other(id.to_owned()),
        }
    }
}

impl<O: std::fmt::Display> std::fmt::Display for AnyResource<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnyResource::Kepler(resource_id) => write!(f, "{}", resource_id),
            AnyResource::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for AnyResource<UriString> {
    type Err = KRIParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("kepler:") {
            Ok(AnyResource::Kepler(s.parse()?))
        } else {
            Ok(AnyResource::Other(s.parse()?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        let res: ResourceId = "kepler:ens:example.eth://orbit0/kv/path/to/image.jpg"
            .parse()
            .unwrap();

        assert_eq!("ens:example.eth", res.orbit().suffix());
        assert_eq!("did:ens:example.eth", res.orbit().did());
        assert_eq!("orbit0", res.orbit().name());
        assert_eq!("kv", res.service().unwrap());
        assert_eq!("/path/to/image.jpg", res.path().unwrap());
        assert_eq!(None, res.fragment().as_ref());

        let res2: ResourceId = "kepler:ens:example.eth://orbit0#peer".parse().unwrap();

        assert_eq!("ens:example.eth", res2.orbit().suffix());
        assert_eq!("did:ens:example.eth", res2.orbit().did());
        assert_eq!("orbit0", res2.orbit().name());
        assert_eq!(None, res2.service());
        assert_eq!(None, res2.path());
        assert_eq!("peer", res2.fragment().unwrap());

        let res3: ResourceId = "kepler:ens:example.eth://orbit0/kv#list".parse().unwrap();

        assert_eq!("kv", res3.service().unwrap());
        assert_eq!("/", res3.path().unwrap());
        assert_eq!("list", res3.fragment().unwrap());

        let res4: ResourceId = "kepler:ens:example.eth://orbit0/kv/#list".parse().unwrap();

        assert_eq!("kv", res4.service().unwrap());
        assert_eq!("/", res4.path().unwrap());
        assert_eq!("list", res4.fragment().unwrap());
    }

    #[test]
    fn failures() {
        let no_suffix: Result<ResourceId, _> = "kepler:://orbit0/kv/path/to/image.jpg".parse();
        assert!(no_suffix.is_err());

        let invalid_name: Result<ResourceId, _> =
            "kepler:ens:example.eth://or:bit0/kv/path/to/image.jpg".parse();
        assert!(invalid_name.is_err());
    }

    #[test]
    fn roundtrip() {
        let resource_uri: String = "kepler:ens:example.eth://orbit0/kv/prefix#list".into();
        let res4: ResourceId = resource_uri.parse().unwrap();
        assert_eq!(resource_uri, res4.to_string());
    }
}
