use crate::resource::ResourceId;
use anyhow::Result;
use cacao_zcap::CacaoZcapExtraProps;
use didkit::DID_METHODS;
use libipld::{
    cbor::DagCborCodec,
    codec::{Decode, Encode},
    error::Error as IpldError,
    json::DagJsonCodec,
    Ipld,
};
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{Cursor, Read, Seek, Write},
};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct InvProps {
    pub invocation_target: ResourceId,
    #[serde(flatten)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_fields: Option<HashMap<String, Value>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Delegation(ssi::zcap::Delegation<(), CacaoZcapExtraProps>);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Invocation(ssi::zcap::Invocation<InvProps>);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Revocation(ssi::zcap::Invocation<InvProps>);

impl Delegation {
    pub fn resource(&self) -> ResourceId {
        todo!()
    }
    pub fn delegator(&self) -> &[u8] {
        todo!()
    }
    pub fn delegate(&self) -> &[u8] {
        todo!()
    }
}

impl Invocation {
    pub fn resource(&self) -> ResourceId {
        todo!()
    }
    pub fn invoker(&self) -> &[u8] {
        todo!()
    }
}

impl Revocation {
    pub fn revoked(&self) -> &[u8] {
        todo!()
    }
    pub fn revoker(&self) -> &[u8] {
        todo!()
    }
}

macro_rules! impl_encode_dagcbor {
    ($type:ident) => {
        impl Encode<DagCborCodec> for $type {
            fn encode<W>(&self, c: DagCborCodec, w: &mut W) -> Result<(), IpldError>
            where
                W: Write,
            {
                // HACK transliterate via serde -> json -> ipld -> cbor
                Ipld::decode(DagJsonCodec, &mut Cursor::new(serde_json::to_vec(self)?))?
                    .encode(c, w)
            }
        }
    };
}

macro_rules! impl_decode_dagcbor {
    ($type:ident) => {
        impl Decode<DagCborCodec> for $type {
            fn decode<R>(c: DagCborCodec, r: &mut R) -> Result<Self, IpldError>
            where
                R: Read + Seek,
            {
                // HACK transliterate via cbor -> ipld -> json -> serde
                let mut b: Vec<u8> = vec![];
                Ipld::decode(c, r)?.encode(DagJsonCodec, &mut b)?;
                Ok(serde_json::from_slice(&b)?)
            }
        }
    };
}

impl_encode_dagcbor!(Delegation);
impl_encode_dagcbor!(Invocation);
impl_encode_dagcbor!(Revocation);

impl_decode_dagcbor!(Delegation);
impl_decode_dagcbor!(Invocation);
impl_decode_dagcbor!(Revocation);

#[derive(thiserror::Error, Debug)]
pub enum HeaderFromReqError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    B64(#[from] base64::DecodeError),
}

macro_rules! impl_fromreq {
    ($type:ident, $name:tt) => {
        #[rocket::async_trait]
        impl<'r> FromRequest<'r> for $type {
            type Error = HeaderFromReqError;
            async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
                match request.headers().get_one($name).map(|b64| {
                    base64::decode_config(b64, base64::URL_SAFE)
                        .map_err(HeaderFromReqError::from)
                        .and_then(|s| Ok(serde_json::from_slice(&s)?))
                }) {
                    Some(Ok(item)) => Outcome::Success(item),
                    Some(Err(e)) => Outcome::Failure((Status::Unauthorized, e)),
                    None => Outcome::Forward(()),
                }
            }
        }
    };
}

impl_fromreq!(Delegation, "Authorization");
impl_fromreq!(Invocation, "Authorization");

pub trait CapNode {
    fn id(&self) -> Vec<u8>;
    fn parents(&self) -> NestedDelegationIter;
    fn parent_ids(&self) -> NestedIdIter;
}

fn uuid_bytes_or_str(s: &str) -> Vec<u8> {
    uuid::Uuid::parse_str(
        s.strip_prefix("urn:uuid:")
            .or_else(|| s.strip_prefix("uuid:"))
            .unwrap_or(s),
    )
    .map(|u| u.as_bytes().to_vec())
    .unwrap_or(s.as_bytes().to_vec())
}

macro_rules! impl_capnode {
    ($type:ident) => {
        impl CapNode for $type {
            fn id(&self) -> Vec<u8> {
                use ssi::vc::URI;
                match &self.0.id {
                    URI::String(ref u) => uuid_bytes_or_str(u),
                }
            }
            fn parents(&self) -> NestedDelegationIter {
                NestedDelegationIter(ParentIter::new(self.0.proof.as_ref()))
            }
            fn parent_ids(&self) -> NestedIdIter {
                NestedIdIter(ParentIter::new(self.0.proof.as_ref()))
            }
        }
    };
}

impl_capnode!(Delegation);
impl_capnode!(Invocation);
impl_capnode!(Revocation);

pub struct ParentIter<'a>(Option<&'a Value>);

impl<'a> ParentIter<'a> {
    pub fn new(proof: Option<&'a ssi::vc::Proof>) -> Self {
        Self(
            proof
                .and_then(|p| p.property_set.as_ref())
                .and_then(|ps| ps.get("capabilityChain"))
                .and_then(|chain| match chain {
                    Value::Array(caps) => caps.last(),
                    _ => None,
                }),
        )
    }
}

impl<'a> Iterator for ParentIter<'a> {
    type Item = &'a Value;
    fn next(&mut self) -> Option<Self::Item> {
        match self.0 {
            Some(c) => {
                self.0 = c
                    .as_object()
                    .and_then(|o| o.get("proof"))
                    .and_then(|p| p.get("capabilityChain"))
                    .and_then(|c| c.get(0));
                Some(c)
            }
            None => None,
        }
    }
}

pub struct NestedDelegationIter<'a>(ParentIter<'a>);

impl<'a> Iterator for NestedDelegationIter<'a> {
    type Item = Delegation;
    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .next()
            .and_then(|p| serde_json::from_value(p.clone()).ok())
    }
}

pub struct NestedIdIter<'a>(ParentIter<'a>);

impl<'a> Iterator for NestedIdIter<'a> {
    type Item = Vec<u8>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .next()
            .and_then(|p| p.get("id"))
            .and_then(|f| f.as_str())
            .map(|id| uuid_bytes_or_str(id))
    }
}

#[rocket::async_trait]
pub trait Verifiable {
    async fn verify(&self) -> Result<()>;
}

#[rocket::async_trait]
impl Verifiable for Delegation {
    async fn verify(&self) -> Result<()> {
        if let Some(e) = self
            .0
            .verify(Default::default(), DID_METHODS.to_resolver())
            .await
            .errors
            .into_iter()
            .next()
        {
            Err(anyhow!(e))
        } else {
            Ok(())
        }
    }
}

#[rocket::async_trait]
impl Verifiable for Invocation {
    async fn verify(&self) -> Result<()> {
        if let Some(e) = self
            .0
            .verify_signature(Default::default(), DID_METHODS.to_resolver())
            .await
            .errors
            .into_iter()
            .next()
        {
            Err(anyhow!(e))
        } else {
            Ok(())
        }
    }
}

#[rocket::async_trait]
impl Verifiable for Revocation {
    async fn verify(&self) -> Result<()> {
        if let Some(e) = self
            .0
            .verify_signature(Default::default(), DID_METHODS.to_resolver())
            .await
            .errors
            .into_iter()
            .next()
        {
            Err(anyhow!(e))
        } else {
            Ok(())
        }
    }
}

#[test]
async fn basic() -> Result<()> {
    let inv_str = r#"{"@context":["https://w3id.org/security/v2",{"capabilityAction":{"@id":"sec:capabilityAction","@type":"@json"}}],"id":"uuid:8097ab5c-ebd6-4924-b659-5f8009429e4d","invocationTarget":"kepler:pkh:eip155:1:0x3401fBE360502F420D5c27CB8AED88E86cc4a726://default/ipfs/#list","proof":{"type":"Ed25519Signature2018","proofPurpose":"capabilityInvocation","verificationMethod":"did:key:z6MkuMN5NfBrN6YbGjzsc5ekSQBVGut3Q6inc8aEtY2AoHZj#z6MkuMN5NfBrN6YbGjzsc5ekSQBVGut3Q6inc8aEtY2AoHZj","created":"2022-03-21T13:59:14.455Z","jws":"eyJhbGciOiJFZERTQSIsImNyaXQiOlsiYjY0Il0sImI2NCI6ZmFsc2V9..ybqGJAhCtAPE97cZTLLvX5f5IzJtZLaCmrYAGosckwt9MT5A-ZRQfcZsdwrDUGND5lSTAIAvxWjCOvtMA1RVCw","capability":"kepler:pkh:eip155:1:0x3401fBE360502F420D5c27CB8AED88E86cc4a726://default"}}"#;
    let inv: Invocation = serde_json::from_str(inv_str)?;
    let res = inv
        .0
        .verify_signature(None, DID_METHODS.to_resolver())
        .await;
    assert!(res.errors.is_empty());
    Ok(())
}
