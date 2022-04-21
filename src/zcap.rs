use lib::{
    didkit::DID_METHODS,
    resource::ResourceId,
    ssi::vc::{Proof, URI},
    zcap::{
        KeplerDelegation as InnerDelegation, KeplerInvocation as InnerInvocation, Verifiable,
        VerificationResult,
    },
};
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
use serde::{de::Error, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{
    convert::{TryFrom, TryInto},
    io::{Cursor, Read, Seek, Write},
    str::FromStr,
};

#[derive(Debug, Serialize, Clone)]
pub struct Delegation(InnerDelegation);

#[derive(Debug, Serialize, Clone)]
pub struct Invocation(InnerInvocation);

#[derive(Debug, Serialize, Clone)]
pub struct Revocation(InnerInvocation);

// NOTE many of these methods contain .unwraps
// these types can only be constructed via checked conversion,
// to maintain the invariants that make these .unwraps work
impl Delegation {
    pub fn resource(&self) -> ResourceId {
        self.0.property_set.invocation_target.parse().unwrap()
    }
    pub fn delegator(&self) -> &[u8] {
        // TODO this is the full did vm not the did itself
        self.0
            .proof
            .as_ref()
            .and_then(|p| p.creator.as_ref())
            .map(|c| c.as_bytes())
            .unwrap()
    }
    pub fn delegate(&self) -> &str {
        self.0
            .invoker
            .as_ref()
            .map(|i| match i {
                URI::String(ref s) => s,
            })
            .unwrap()
    }
}

impl Invocation {
    pub fn resource(&self) -> &ResourceId {
        &self.0.property_set.invocation_target
    }
    pub fn invoker(&self) -> &[u8] {
        self.0
            .proof
            .as_ref()
            .and_then(|p| p.creator.as_ref())
            .map(|c| c.as_bytes())
            .unwrap()
    }
}

impl Revocation {
    pub fn revoked(&self) -> &[u8] {
        &self
            .0
            .property_set
            .invocation_target
            .path()
            .unwrap()
            .as_bytes()
    }
    pub fn revoker(&self) -> &[u8] {
        self.0
            .proof
            .as_ref()
            .and_then(|p| p.creator.as_ref())
            .map(|c| c.as_bytes())
            .unwrap()
    }
}

macro_rules! impl_deref {
    ($type:ident, $target:ident) => {
        impl std::ops::Deref for $type {
            type Target = $target;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
}

impl_deref!(Delegation, InnerDelegation);
impl_deref!(Invocation, InnerInvocation);
impl_deref!(Revocation, InnerInvocation);

#[derive(thiserror::Error, Debug)]
pub enum DelegationError {
    #[error("Invalid Resource")]
    InvalidResource,
    #[error("Missing Delegator")]
    MissingDelegator,
    #[error("Missing Delegate")]
    MissingDelegate,
}

impl TryFrom<InnerDelegation> for Delegation {
    type Error = DelegationError;
    fn try_from(d: InnerDelegation) -> Result<Self, Self::Error> {
        match (
            ResourceId::from_str(&d.property_set.invocation_target),
            &d.proof.as_ref().and_then(|p| p.creator.as_ref()),
            &d.invoker,
        ) {
            (Err(_), _, _) => Err(DelegationError::InvalidResource),
            (_, None, _) => Err(DelegationError::MissingDelegator),
            (_, _, None) => Err(DelegationError::MissingDelegate),
            _ => Ok(Self(d)),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum InvocationError {
    #[error("Missing Invoker")]
    MissingInvoker,
}

impl TryFrom<InnerInvocation> for Invocation {
    type Error = InvocationError;
    fn try_from(i: InnerInvocation) -> Result<Self, Self::Error> {
        match i.proof.as_ref().and_then(|p| p.creator.as_ref()) {
            None => Err(InvocationError::MissingInvoker),
            _ => Ok(Self(i)),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum RevocationError {
    #[error("Invalid Target")]
    InvalidTarget,
    #[error("Missing Revoker")]
    MissingRevoker,
}

impl TryFrom<InnerInvocation> for Revocation {
    type Error = RevocationError;
    fn try_from(i: InnerInvocation) -> Result<Self, Self::Error> {
        match (
            i.property_set.invocation_target.path(),
            i.proof.as_ref().and_then(|p| p.creator.as_ref()),
        ) {
            (None, _) => Err(RevocationError::InvalidTarget),
            (_, None) => Err(RevocationError::MissingRevoker),
            _ => Ok(Self(i)),
        }
    }
}

impl TryFrom<Invocation> for Revocation {
    type Error = RevocationError;
    fn try_from(i: Invocation) -> Result<Self, Self::Error> {
        i.0.try_into()
    }
}

macro_rules! impl_deserialize {
    ($origin:ident, $type:ident) => {
        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                $origin::deserialize(deserializer)?
                    .try_into()
                    .map_err(D::Error::custom)
            }
        }
    };
}

impl_deserialize!(InnerDelegation, Delegation);
impl_deserialize!(InnerInvocation, Invocation);
impl_deserialize!(InnerInvocation, Revocation);

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
    pub fn new(proof: Option<&'a Proof>) -> Self {
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

macro_rules! impl_verifiable {
    ($type:ident) => {
        #[rocket::async_trait]
        impl Verifiable for $type {
            async fn verify(&self) -> VerificationResult {
                Verifiable::verify(&self.0).await
            }
        }
    };
}

impl_verifiable!(Delegation);
impl_verifiable!(Invocation);
impl_verifiable!(Revocation);

#[test]
async fn basic() -> anyhow::Result<()> {
    let inv_str = r#"{"@context":["https://w3id.org/security/v2",{"capabilityAction":{"@id":"sec:capabilityAction","@type":"@json"}}],"id":"uuid:8097ab5c-ebd6-4924-b659-5f8009429e4d","invocationTarget":"kepler:pkh:eip155:1:0x3401fBE360502F420D5c27CB8AED88E86cc4a726://default/ipfs/#list","proof":{"type":"Ed25519Signature2018","proofPurpose":"capabilityInvocation","verificationMethod":"did:key:z6MkuMN5NfBrN6YbGjzsc5ekSQBVGut3Q6inc8aEtY2AoHZj#z6MkuMN5NfBrN6YbGjzsc5ekSQBVGut3Q6inc8aEtY2AoHZj","created":"2022-03-21T13:59:14.455Z","jws":"eyJhbGciOiJFZERTQSIsImNyaXQiOlsiYjY0Il0sImI2NCI6ZmFsc2V9..ybqGJAhCtAPE97cZTLLvX5f5IzJtZLaCmrYAGosckwt9MT5A-ZRQfcZsdwrDUGND5lSTAIAvxWjCOvtMA1RVCw","capability":"kepler:pkh:eip155:1:0x3401fBE360502F420D5c27CB8AED88E86cc4a726://default"}}"#;
    let inv: Invocation = serde_json::from_str(inv_str)?;
    let res = inv
        .0
        .verify_signature(None, DID_METHODS.to_resolver())
        .await;
    assert!(res.errors.is_empty());
    Ok(())
}
