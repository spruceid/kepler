use crate::resource::{AnyResource, ResourceCapErr, ResourceId};
use cacaos::v2::{common::CommonCacao, varsig::either::EitherSignature, Cacao};
use iri_string::types::{UriStr, UriString};
use ssi::ucan::{
    capabilities::*,
    common::Common,
    jwt::{Jwt, UcanDecode},
    Ucan,
};
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

pub trait Resources<'a, RO: 'a = &'a UriStr, NB: 'a = serde_json::Value> {
    type RI;
    type Iter: Iterator<Item = (RO, &'a BTreeMap<Ability, NotaBeneCollection<NB>>)>;
    fn grants(&'a self) -> Self::Iter;
    fn resources(&'a self) -> Map<Self::Iter, fn(<Self::Iter as Iterator>::Item) -> RO> {
        self.grants().map(|(r, _)| r)
    }
}

pub type KeplerDelegation = CommonCacao;

impl HeaderEncode for KeplerDelegation {
    fn encode(&self) -> Result<String, EncodingError> {
        Ok(match self.signature().sig() {
            EitherSignature::A(_) => {
                base64::encode_config(serde_ipld_dagcbor::to_vec(self)?, base64::URL_SAFE)
            }
            EitherSignature::B(_) => self.serialize_jwt()?.ok_or(EncodingError::NotAJwt)?,
        })
    }

    fn decode(s: &str) -> Result<(Self, Vec<u8>), EncodingError> {
        Ok(if s.contains('.') {
            (
                <Ucan<Common> as UcanDecode<Jwt>>::decode(s)?.try_into()?,
                s.as_bytes().to_vec(),
            )
        } else {
            let v = base64::decode_config(s, base64::URL_SAFE)?;
            (serde_ipld_dagcbor::from_slice(&v)?, v)
        })
    }
}

impl<'a, NB: 'a, RO: 'a, F: 'a, S: 'a> Resources<'a, RO, NB> for Cacao<S, F, NB>
where
    Capabilities<NB>: Resources<'a, RO, NB>,
{
    type RI = <Capabilities<NB> as Resources<'a, RO, NB>>::RI;
    type Iter = <Capabilities<NB> as Resources<'a, RO, NB>>::Iter;
    fn grants(&'a self) -> Self::Iter {
        self.capabilities().grants()
    }
}

impl<'a, NB: 'a> Resources<'a, ResourceId, NB> for Capabilities<NB> {
    type RI = &'a UriString;
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
    type RI = &'a UriString;
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

impl<'a, NB: 'a> Resources<'a, AnyResource<&'a UriStr>, NB> for Capabilities<NB> {
    type RI = &'a UriString;
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

pub fn delegation_from_bytes(b: &[u8]) -> Result<KeplerDelegation, EncodingError> {
    match serde_ipld_dagcbor::from_slice(b) {
        Ok(cacao) => Ok(cacao),
        Err(_) => Ok(
            <Ucan<Common> as UcanDecode<Jwt>>::decode(&String::from_utf8_lossy(b))?.try_into()?,
        ),
    }
}

pub type KeplerInvocation = CommonCacao;

#[derive(Debug, Clone)]
pub enum KeplerRevocation {
    Cacao(CommonCacao),
}

impl HeaderEncode for KeplerRevocation {
    fn encode(&self) -> Result<String, EncodingError> {
        match self {
            Self::Cacao(c) => Ok(base64::encode_config(
                serde_ipld_dagcbor::to_vec(&c)?,
                base64::URL_SAFE,
            )),
        }
    }
    fn decode(s: &str) -> Result<(Self, Vec<u8>), EncodingError> {
        let v = base64::decode_config(s, base64::URL_SAFE)?;
        Ok((Self::Cacao(serde_ipld_dagcbor::from_slice(&v)?), v))
    }
}

// pub async fn make_invocation(
//     invocation_target: Vec<ResourceId>,
//     delegation: Cid,
//     jwk: &JWK,
//     verification_method: String,
//     expiration: f64,
//     not_before: Option<f64>,
//     nonce: Option<String>,
// ) -> Result<Ucan, InvocationError> {
//     Ok(Payload {
//         issuer: verification_method.clone(),
//         audience: verification_method,
//         not_before: not_before.map(NumericDate::try_from_seconds).transpose()?,
//         expiration: NumericDate::try_from_seconds(expiration)?,
//         nonce: Some(nonce.unwrap_or_else(|| format!("urn:uuid:{}", Uuid::new_v4()))),
//         facts: None,
//         proof: vec![delegation],
//         attenuation: invocation_target
//             .into_iter()
//             .map(|t| t.try_into())
//             .collect::<Result<Vec<ssi::ucan::capabilities::Capabilities>, _>>()?,
//     }
//     .sign(jwk.get_algorithm().unwrap_or_default(), jwk)?)
// }

#[derive(Debug, thiserror::Error)]
pub enum InvocationError {
    #[error(transparent)]
    ResourceCap(#[from] ResourceCapErr),
    #[error(transparent)]
    NumericDateConversion(#[from] ssi::jwt::NumericDateConversionError),
    #[error(transparent)]
    UCAN(#[from] ssi::ucan::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum EncodingError {
    #[error(transparent)]
    UCAN(#[from] ssi::ucan::Error),
    #[error(transparent)]
    CacaoError(#[from] cacaos::v2::common::Error),
    #[error(transparent)]
    ToIpldError(#[from] serde_ipld_dagcbor::EncodeError<std::collections::TryReserveError>),
    #[error(transparent)]
    Base64(#[from] base64::DecodeError),
    #[error(transparent)]
    FromIpldError(#[from] serde_ipld_dagcbor::DecodeError<std::convert::Infallible>),
    #[error("CACAO not a JWT")]
    NotAJwt,
}

pub enum CapabilitiesQuery {
    All,
}
