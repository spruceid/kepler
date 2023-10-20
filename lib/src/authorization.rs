use crate::resource::{AnyResource, ResourceId};
use cacaos::{
    common::{CommonCacao, Signature},
    ucan_cacao::UcanCacao,
};
use iri_string::types::{UriStr, UriString};
use ssi::ucan::{capabilities::*, jose, jwt::Jwt, Revocation as URevocation, Ucan, UcanDecode};
use std::{
    collections::BTreeMap,
    iter::{FilterMap, Map},
};

pub use libipld::Cid;

pub trait HeaderEncode {
    fn encode(&self) -> Result<String, EncodingError>;
    fn decode(s: &str) -> Result<(Self, Vec<u8>), EncodingError>
    where
        Self: Sized;
}

pub type ResourceIter<I, O> = Map<I, fn(<I as Iterator>::Item) -> O>;

pub trait Resources<'a, RO: 'a = &'a UriStr, NB: 'a = serde_json::Value> {
    type Iter: Iterator<Item = (RO, &'a BTreeMap<Ability, NotaBeneCollection<NB>>)>;
    fn grants(&'a self) -> Self::Iter;
    fn resources(&'a self) -> ResourceIter<Self::Iter, RO> {
        self.grants().map(|(r, _)| r)
    }
}

pub type Delegation = CommonCacao<BTreeMap<String, serde_json::Value>, serde_json::Value>;

impl HeaderEncode for Delegation {
    fn encode(&self) -> Result<String, EncodingError> {
        Ok(match self.signature() {
            Signature::Ucan(_) => self.serialize_jwt()?.ok_or(EncodingError::NotAJwt)?,
            _ => base64::encode_config(serde_ipld_dagcbor::to_vec(self)?, base64::URL_SAFE),
        })
    }

    fn decode(s: &str) -> Result<(Self, Vec<u8>), EncodingError> {
        Ok(if s.contains('.') {
            (
                UcanCacao::try_from(<Ucan as UcanDecode<Jwt>>::decode(s)?)?.into(),
                s.as_bytes().to_vec(),
            )
        } else {
            let v = base64::decode_config(s, base64::URL_SAFE)?;
            (serde_ipld_dagcbor::from_slice(&v)?, v)
        })
    }
}

impl<'a, U: 'a, NB: 'a, RO: 'a, W: 'a> Resources<'a, RO, NB> for CommonCacao<U, NB, W>
where
    Capabilities<NB>: Resources<'a, RO, NB>,
{
    type Iter = <Capabilities<NB> as Resources<'a, RO, NB>>::Iter;
    fn grants(&'a self) -> Self::Iter {
        self.capabilities().grants()
    }
}

impl<'a, NB: 'a> Resources<'a, ResourceId, NB> for Capabilities<NB> {
    type Iter = FilterMap<
        std::collections::btree_map::Iter<'a, UriString, BTreeMap<Ability, NotaBeneCollection<NB>>>,
        fn(
            (&'a UriString, &'a BTreeMap<Ability, NotaBeneCollection<NB>>),
        ) -> Option<(ResourceId, &'a BTreeMap<Ability, NotaBeneCollection<NB>>)>,
    >;
    fn grants(&'a self) -> Self::Iter {
        self.abilities()
            .iter()
            .filter_map(|(r, a)| r.try_into().map(|k| (k, a)).ok())
    }
}

impl<'a, NB: 'a> Resources<'a, &'a UriStr, NB> for Capabilities<NB> {
    type Iter = Map<
        std::collections::btree_map::Iter<'a, UriString, BTreeMap<Ability, NotaBeneCollection<NB>>>,
        fn(
            (&'a UriString, &'a BTreeMap<Ability, NotaBeneCollection<NB>>),
        ) -> (&'a UriStr, &'a BTreeMap<Ability, NotaBeneCollection<NB>>),
    >;
    fn grants(&'a self) -> Self::Iter {
        self.abilities().iter().map(|(r, a)| (r.as_ref(), a))
    }
}

impl<'a, NB: 'a> Resources<'a, AnyResource, NB> for Capabilities<NB> {
    type Iter = Map<
        std::collections::btree_map::Iter<'a, UriString, BTreeMap<Ability, NotaBeneCollection<NB>>>,
        fn(
            (&'a UriString, &'a BTreeMap<Ability, NotaBeneCollection<NB>>),
        ) -> (AnyResource, &'a BTreeMap<Ability, NotaBeneCollection<NB>>),
    >;
    fn grants(&'a self) -> Self::Iter {
        self.abilities().iter().map(|(r, a)| (r.into(), a))
    }
}

impl<'a, NB: 'a> Resources<'a, AnyResource<&'a UriStr>, NB> for Capabilities<NB> {
    type Iter = Map<
        std::collections::btree_map::Iter<'a, UriString, BTreeMap<Ability, NotaBeneCollection<NB>>>,
        fn(
            (&'a UriString, &'a BTreeMap<Ability, NotaBeneCollection<NB>>),
        ) -> (
            AnyResource<&'a UriStr>,
            &'a BTreeMap<Ability, NotaBeneCollection<NB>>,
        ),
    >;
    fn grants(&'a self) -> Self::Iter {
        self.abilities().iter().map(|(r, a)| (r.into(), a))
    }
}

pub fn delegation_from_bytes(b: &[u8]) -> Result<Delegation, EncodingError> {
    match serde_ipld_dagcbor::from_slice(b) {
        Ok(cacao) => Ok(cacao),
        Err(_) => Ok(<Ucan as UcanDecode<Jwt>>::decode(&String::from_utf8_lossy(b))?.try_into()?),
    }
}

pub type Invocation = CommonCacao<BTreeMap<String, serde_json::Value>, serde_json::Value>;

pub type Revocation = URevocation;

impl HeaderEncode for Revocation {
    fn encode(&self) -> Result<String, EncodingError> {
        Ok(base64::encode_config(
            serde_ipld_dagcbor::to_vec(&self)?,
            base64::URL_SAFE,
        ))
    }
    fn decode(s: &str) -> Result<(Self, Vec<u8>), EncodingError> {
        let v = base64::decode_config(s, base64::URL_SAFE)?;
        Ok((serde_ipld_dagcbor::from_slice(&v)?, v))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EncodingError {
    #[error(transparent)]
    UCAN(#[from] ssi::ucan::jwt::DecodeError<jose::Error>),
    #[error(transparent)]
    CacaoError(#[from] cacaos::common::Error),
    #[error(transparent)]
    ToIpldError(#[from] serde_ipld_dagcbor::EncodeError<std::collections::TryReserveError>),
    #[error(transparent)]
    Base64(#[from] base64::DecodeError),
    #[error(transparent)]
    FromIpldError(#[from] serde_ipld_dagcbor::DecodeError<std::convert::Infallible>),
    #[error("CACAO not a JWT")]
    NotAJwt,
}

impl From<cacaos::v3::ucan_cacao::Error> for EncodingError {
    fn from(e: cacaos::v3::ucan_cacao::Error) -> EncodingError {
        EncodingError::CacaoError(e.into())
    }
}

pub enum CapabilitiesQuery {
    All,
}
