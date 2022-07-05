use crate::resource::{ResourceCapErr, ResourceId};
use cacaos::siwe_cacao::SiweCacao;
use didkit::DID_METHODS;
use libipld::{cbor::DagCborCodec, prelude::*, Cid};
use ssi::{
    jwk::JWK,
    ucan::{Capability, Payload, Ucan},
    vc::{NumericDate, URI},
};
use uuid::Uuid;

pub trait HeaderEncode {
    fn encode(&self) -> Result<String, EncodingError>;
    fn decode(s: &str) -> Result<Self, EncodingError>
    where
        Self: Sized;
}

#[derive(Clone, Debug)]
pub enum KeplerDelegation {
    Ucan(Ucan),
    Cacao(SiweCacao),
}

impl HeaderEncode for KeplerDelegation {
    fn encode(&self) -> Result<String, EncodingError> {
        Ok(match self {
            Self::Ucan(u) => u.encode()?,
            Self::Cacao(c) => base64::encode_config(DagCborCodec.encode(&c)?, base64::URL_SAFE),
        })
    }

    fn decode(s: &str) -> Result<Self, EncodingError> {
        Ok(if s.contains(".") {
            Self::Ucan(Ucan::decode(s)?)
        } else {
            Self::Cacao(DagCborCodec.decode(&base64::decode_config(s, base64::URL_SAFE)?)?)
        })
    }
}

// turn everything into url safe, b64-cacao or jwt

pub type KeplerInvocation = Ucan;

impl HeaderEncode for KeplerInvocation {
    fn encode(&self) -> Result<String, EncodingError> {
        Ok(self.encode()?)
    }
    fn decode(s: &str) -> Result<Self, EncodingError> {
        Ok(Self::decode(s)?)
    }
}

#[derive(Debug, Clone)]
pub enum KeplerRevocation {
    Cacao(SiweCacao),
}

impl HeaderEncode for KeplerRevocation {
    fn encode(&self) -> Result<String, EncodingError> {
        match self {
            Self::Cacao(c) => Ok(base64::encode_config(
                DagCborCodec.encode(&c)?,
                base64::URL_SAFE,
            )),
        }
    }
    fn decode(s: &str) -> Result<Self, EncodingError> {
        Ok(Self::Cacao(
            DagCborCodec.decode(&base64::decode_config(s, base64::URL_SAFE)?)?,
        ))
    }
}

pub async fn make_invocation(
    invocation_target: ResourceId,
    delegation: Cid,
    jwk: &JWK,
    verification_method: String,
    audience: String,
    expiration: f64,
    not_before: Option<f64>,
    nonce: Option<String>,
) -> Result<Ucan, InvocationError> {
    Ok(Payload {
        issuer: verification_method,
        audience,
        not_before: not_before.map(NumericDate::try_from_seconds).transpose()?,
        expiration: NumericDate::try_from_seconds(expiration)?,
        nonce: Some(nonce.unwrap_or_else(|| format!("urn:uuid:{}", Uuid::new_v4()))),
        facts: None,
        proof: vec![delegation],
        attenuation: vec![invocation_target.try_into()?],
    }
    .sign(jwk.algorithm.unwrap_or_else(Default::default), jwk)?)
}

#[derive(Debug, thiserror::Error)]
pub enum InvocationError {
    #[error(transparent)]
    ResourceCapErr(#[from] ResourceCapErr),
    #[error(transparent)]
    SSIError(#[from] ssi::error::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum EncodingError {
    #[error(transparent)]
    SSIError(#[from] ssi::error::Error),
    #[error(transparent)]
    IpldError(#[from] libipld::error::Error),
    #[error(transparent)]
    Base64(#[from] base64::DecodeError),
}
