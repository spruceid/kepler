use crate::resource::{ResourceCapErr, ResourceId};
use cacaos::siwe_cacao::SiweCacao;
use libipld::{cbor::DagCborCodec, prelude::*};
use ssi::{
    jwk::JWK,
    ucan::{Payload, Ucan},
    vc::NumericDate,
};
use uuid::Uuid;

pub use libipld::Cid;

pub trait HeaderEncode {
    fn encode(&self) -> Result<String, EncodingError>;
    fn decode(s: &str) -> Result<(Self, Vec<u8>), EncodingError>
    where
        Self: Sized;
}

#[derive(Clone, Debug)]
pub enum KeplerDelegation {
    Ucan(Box<Ucan>),
    Cacao(Box<SiweCacao>),
}

impl HeaderEncode for KeplerDelegation {
    fn encode(&self) -> Result<String, EncodingError> {
        use std::ops::Deref;
        Ok(match self {
            Self::Ucan(u) => u.encode()?,
            Self::Cacao(c) => {
                base64::encode_config(DagCborCodec.encode(c.deref())?, base64::URL_SAFE)
            }
        })
    }

    fn decode(s: &str) -> Result<(Self, Vec<u8>), EncodingError> {
        Ok(if s.contains('.') {
            (
                Self::Ucan(Box::new(Ucan::decode(s)?)),
                s.as_bytes().to_vec(),
            )
        } else {
            let v = base64::decode_config(s, base64::URL_SAFE)?;
            (Self::Cacao(Box::new(DagCborCodec.decode(&v)?)), v)
        })
    }
}

impl KeplerDelegation {
    pub fn from_bytes(b: &[u8]) -> Result<Self, EncodingError> {
        match DagCborCodec.decode(b) {
            Ok(cacao) => Ok(Self::Cacao(Box::new(cacao))),
            Err(_) => Ok(Self::Ucan(Box::new(Ucan::decode(
                &String::from_utf8_lossy(b),
            )?))),
        }
    }
}

// turn everything into url safe, b64-cacao or jwt

pub type KeplerInvocation = Ucan;

impl HeaderEncode for KeplerInvocation {
    fn encode(&self) -> Result<String, EncodingError> {
        Ok(self.encode()?)
    }
    fn decode(s: &str) -> Result<(Self, Vec<u8>), EncodingError> {
        Ok((Self::decode(s)?, s.as_bytes().to_vec()))
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
    fn decode(s: &str) -> Result<(Self, Vec<u8>), EncodingError> {
        let v = base64::decode_config(s, base64::URL_SAFE)?;
        Ok((Self::Cacao(DagCborCodec.decode(&v)?), v))
    }
}

pub async fn make_invocation(
    invocation_target: Vec<ResourceId>,
    delegation: Cid,
    jwk: &JWK,
    verification_method: String,
    expiration: f64,
    not_before: Option<f64>,
    nonce: Option<String>,
) -> Result<Ucan, InvocationError> {
    Ok(Payload {
        issuer: verification_method.clone(),
        audience: verification_method,
        not_before: not_before.map(NumericDate::try_from_seconds).transpose()?,
        expiration: NumericDate::try_from_seconds(expiration)?,
        nonce: Some(nonce.unwrap_or_else(|| format!("urn:uuid:{}", Uuid::new_v4()))),
        facts: None,
        proof: vec![delegation],
        attenuation: invocation_target
            .into_iter()
            .map(|t| t.try_into())
            .collect::<Result<Vec<ssi::ucan::Capability>, _>>()?,
    }
    .sign(jwk.get_algorithm().unwrap_or_default(), jwk)?)
}

#[derive(Debug, thiserror::Error)]
pub enum InvocationError {
    #[error(transparent)]
    ResourceCap(#[from] ResourceCapErr),
    #[error(transparent)]
    NumericDateConversion(#[from] ssi::jwt::NumericDateConversionError),
    #[error(transparent)]
    UCAN(#[from] ssi::ucan::error::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum EncodingError {
    #[error(transparent)]
    SSIError(#[from] ssi::ucan::error::Error),
    #[error(transparent)]
    IpldError(#[from] libipld::error::Error),
    #[error(transparent)]
    Base64(#[from] base64::DecodeError),
}

pub enum CapabilitiesQuery {
    All,
}
