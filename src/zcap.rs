use crate::capabilities::store::decode_root;
use anyhow::Result;
use chrono::{DateTime, Utc};
use lib::{
    didkit::DID_METHODS,
    resource::ResourceId,
    ssi::{vc::{Proof, URI},
cacao_zcap::CacaoZcapExtraProps},
    zcap::{KeplerDelegation as InnerDelegation, KeplerInvocation as InnerInvocation},
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
    pub fn resources(&self) -> Vec<ResourceId> {
        let r = self.resource();
        self.0
            .property_set
            .allowed_action
            .as_ref()
            .map(|aa| {
                aa.into_iter()
                    .map(|a| {
                        r.orbit().clone().to_resource(
                            r.service().map(|s| s.to_string()),
                            r.path().map(|p| p.to_string()),
                            Some(a.to_string()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_else(Vec::new)
    }
    pub fn delegator(&self) -> &str {
        // NOTE this returns the full did VM URL e.g. did:example:alice#alice-key-1
        self.0
            .proof
            .as_ref()
            .and_then(|p| p.verification_method.as_ref())
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
    pub fn invoker(&self) -> &str {
        self.0
            .proof
            .as_ref()
            .and_then(|p| p.verification_method.as_ref())
            .unwrap()
    }
}

impl Revocation {
    pub fn revoked(&self) -> Vec<u8> {
        check_target_is_delegation(&self.0.property_set.invocation_target).unwrap()
    }
    pub fn revoker(&self) -> &str {
        self.0
            .proof
            .as_ref()
            .and_then(|p| p.verification_method.as_ref())
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
            &d.proof
                .as_ref()
                .and_then(|p| p.verification_method.as_ref()),
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
        match i
            .proof
            .as_ref()
            .and_then(|p| p.verification_method.as_ref())
        {
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
            i.proof
                .as_ref()
                .and_then(|p| p.verification_method.as_ref()),
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
    fn root(&self) -> Option<&str>;
}

fn uuid_bytes_or_str(s: &str) -> Vec<u8> {
    uuid::Uuid::parse_str(
        s.strip_prefix("urn:uuid:")
            .or_else(|| s.strip_prefix("uuid:"))
            .unwrap_or(s),
    )
    .map(|u| u.as_bytes().to_vec())
    .unwrap_or_else(|_| s.as_bytes().to_vec())
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
            fn root(&self) -> Option<&str> {
                self.0
                    .proof
                    .as_ref()
                    .and_then(|p| p.property_set.as_ref())
                    .and_then(|ps| ps.get("capabilityChain"))
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.as_str())
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
                .and_then(|p| p.get("capabilityChain"))
                .and_then(|c| c.as_array())
                .and_then(|c| c.last()),
        )
    }
}

impl<'a> Iterator for ParentIter<'a> {
    type Item = &'a Value;
    fn next(&mut self) -> Option<Self::Item> {
        let c = self.0;
        let n = c
            .and_then(|v| v.get("proof"))
            .and_then(|p| p.get("capabilityChain"))
            .and_then(|c| c.as_array())
            .and_then(|c| c.last());
        self.0 = n;
        c
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
            .map(uuid_bytes_or_str)
    }
}

#[rocket::async_trait]
pub trait Verifiable {
    async fn verify(&self, timestamp: Option<DateTime<Utc>>) -> Result<()>;
}

fn check_time(t: Option<&String>) -> Result<Option<DateTime<Utc>>> {
    Ok(t.map(|s| s.parse()).transpose()?)
}

#[rocket::async_trait]
impl Verifiable for Delegation {
    async fn verify(&self, time: Option<DateTime<Utc>>) -> Result<()> {
        let t = time.unwrap_or_else(Utc::now);
        match check_time(self.0.property_set.expires.as_ref())? {
            Some(exp) if t > exp => bail!("Expired"),
            _ => (),
        };
        match check_time(self.0.property_set.valid_from.as_ref())? {
            Some(nbf) if t < nbf => bail!("Not Yet Valid"),
            _ => (),
        };
        if let Some(e) = self
            .0
            .verify(Default::default(), DID_METHODS.to_resolver())
            .await
            .errors
            .into_iter()
            .next()
        {
            bail!(e)
        };
        if self
            .0
            .property_set
            .allowed_action
            .as_ref()
            .map(|a| a.any(|h| h == "host"))
            .unwrap_or(false)
        {
            if Some("Authorize this peer to host your orbit.")
                != self.0.property_set.cacao_zcap_substatement.as_deref()
            {
                bail!("Incorrect Substatement for Host Delegation")
            };
        } else if self.0.property_set.cacao_zcap_substatement.as_deref()
            != Some("Allow access to your Kepler orbit using this session key.")
        {
            bail!("Incorrect Substatement for Delegated Resources")
        };
        if let Some(p) = self.parents().next() {
            if p.delegate() != self.delegator() {
                bail!("Delegator Not Authorized")
            };
            if p.root() != self.root() {
                bail!("Auth root caps do not match")
            };
            if !self
                .resources()
                .iter()
                .all(|r| p.resources().iter().any(|pr| r.extends(pr).is_ok()))
            {
                bail!("Authorization not granted by parent")
            };
            p.verify(Some(t)).await?;
        } else if compare_root_with_issuer(self.root(), self.delegator()).is_err() {
            bail!("Delegator Not Authorized by root Orbit Capability")
        };
        Ok(())
    }
}

#[rocket::async_trait]
impl Verifiable for Invocation {
    async fn verify(&self, time: Option<DateTime<Utc>>) -> Result<()> {
        let t = time.unwrap_or_else(Utc::now);
        match check_time(self.0.property_set.expires.as_ref())? {
            Some(exp) if t > exp => bail!("Expired"),
            _ => (),
        };
        match check_time(self.0.property_set.valid_from.as_ref())? {
            Some(nbf) if t < nbf => bail!("Not Yet Valid"),
            _ => (),
        };
        if let Some(e) = self
            .0
            .verify_signature(Default::default(), DID_METHODS.to_resolver())
            .await
            .errors
            .into_iter()
            .next()
        {
            bail!(e)
        };
        if let Some(p) = self.parents().next() {
            if p.delegate() != self.invoker() {
                bail!("Invoker Not Authorized")
            };
            if p.root() != self.root() {
                bail!("Auth root caps do not match")
            };
            if !p
                .resources()
                .iter()
                .any(|pr| self.resource().extends(pr).is_ok())
            {
                bail!("Authorization not granted by parent")
            };
            p.verify(Some(t)).await?;
        } else if compare_root_with_issuer(self.root(), self.invoker()).is_err() {
            bail!("Invoker Not Authorized by root Orbit Capability")
        };
        Ok(())
    }
}

fn compare_root_with_issuer(root: Option<&str>, vm: &str) -> Result<()> {
    match root.map(decode_root).transpose()?.map(|r| r.did()) {
        Some(r) if r == vm.split_once('#').map(|s| s.0).unwrap_or(vm) => Ok(()),
        _ => Err(anyhow!("Issuer not authorized by Root")),
    }
}

#[rocket::async_trait]
impl Verifiable for Revocation {
    async fn verify(&self, time: Option<DateTime<Utc>>) -> Result<()> {
        let t = time.unwrap_or_else(Utc::now);
        match check_time(self.0.property_set.expires.as_ref())? {
            Some(exp) if t > exp => bail!("Expired"),
            _ => (),
        };
        match check_time(self.0.property_set.valid_from.as_ref())? {
            Some(nbf) if t < nbf => bail!("Not Yet Valid"),
            _ => (),
        };
        if let Some(e) = self
            .0
            .verify_signature(Default::default(), DID_METHODS.to_resolver())
            .await
            .errors
            .into_iter()
            .next()
        {
            bail!(e)
        };
        if let Some(p) = self.parents().next() {
            // TODO we should check if the revoker is the delegate/delegator of ANY
            // of the parent delegations
            if p.delegate() != self.revoker() {
                bail!("Revoker Not Authorized")
            };
            if p.root() != self.root() {
                bail!("Auth root caps do not match")
            };
            p.verify(Some(t)).await?;
        } else if compare_root_with_issuer(self.root(), self.revoker()).is_err() {
            bail!("Revoker Not Authorized by root Orbit Capability")
        };
        Ok(())
    }
}

fn check_target_is_delegation(target: &ResourceId) -> Option<Vec<u8>> {
    match (
        target.service(),
        target
            .path()
            .and_then(|p| p.strip_prefix("/delegations/"))
            .map(uuid_bytes_or_str),
    ) {
        // TODO what exactly do we expect here
        (Some("capabilities"), Some(p)) => Some(p),
        _ => None,
    }
}

#[test]
async fn basic() -> Result<()> {
    let inv_str = r#"{"@context":"https://w3id.org/security/v2","id":"urn:uuid:5daa6422-4636-4009-a13a-a2c2799e66dd","invocationTarget":"kepler:pkh:eip155:1:0xe54Ce520fc6ea6Db0f75A6f66A22db7a427D9bD2://default/kv/#list","proof":{"type":"Ed25519Signature2018","proofPurpose":"capabilityInvocation","verificationMethod":"did:key:z6MkhKej386g7b4RHtDdjGiNKfjDAsTPCDNtCMRKHmdVBAYD#z6MkhKej386g7b4RHtDdjGiNKfjDAsTPCDNtCMRKHmdVBAYD","created":"2022-05-31T14:02:45.153Z","jws":"eyJhbGciOiJFZERTQSIsImNyaXQiOlsiYjY0Il0sImI2NCI6ZmFsc2V9..Otm5jN6Ax7mUStbX8kljvBKyKnDR1-Hg8TO_fAX8Reravq0ePbvim_xBzflQApMEwrfVWVZRKEkue7joVNw5Dw","capabilityChain":["urn:zcap:root:kepler%3Apkh%3Aeip155%3A1%3A0xe54Ce520fc6ea6Db0f75A6f66A22db7a427D9bD2%3A%2F%2Fdefault",{"@context":["https://w3id.org/security/v2","https://demo.didkit.dev/2022/cacao-zcap/contexts/v1.json"],"allowedAction":["put","get","list","del","metadata"],"cacaoPayloadType":"eip4361","cacaoZcapSubstatement":"Allow access to your Kepler orbit using this session key.","expires":"2022-05-31T15:02:45.096Z","id":"urn:uuid:74a482e8-b972-45fe-ae97-a7b62e641642","invocationTarget":"kepler:pkh:eip155:1:0xe54Ce520fc6ea6Db0f75A6f66A22db7a427D9bD2://default/kv","invoker":"did:key:z6MkhKej386g7b4RHtDdjGiNKfjDAsTPCDNtCMRKHmdVBAYD#z6MkhKej386g7b4RHtDdjGiNKfjDAsTPCDNtCMRKHmdVBAYD","parentCapability":"urn:zcap:root:kepler%3Apkh%3Aeip155%3A1%3A0xe54Ce520fc6ea6Db0f75A6f66A22db7a427D9bD2%3A%2F%2Fdefault","proof":{"cacaoSignatureType":"eip191","capabilityChain":["urn:zcap:root:kepler%3Apkh%3Aeip155%3A1%3A0xe54Ce520fc6ea6Db0f75A6f66A22db7a427D9bD2%3A%2F%2Fdefault"],"created":"2022-05-31T14:02:45.096Z","domain":"example2.com","nonce":"TFQrkNtGohC","proofPurpose":"capabilityDelegation","proofValue":"f33c57e2564f51ba267db7843b437592c00d1f9c789d29f06876a8d5aefb21d9477ddefc2ddd765c7aae3acff32a43b3e105f85b571efa4c5feecd7b8b105385d1b","type":"CacaoZcapProof2022","verificationMethod":"did:pkh:eip155:1:0xe54Ce520fc6ea6Db0f75A6f66A22db7a427D9bD2#blockchainAccountId"},"type":"CacaoZcap2022"}]}}"#;
    let inv: Invocation = serde_json::from_str(inv_str)?;
    inv.verify(Some("2022-05-31T14:02:45.836Z".parse()?))
        .await?;
    assert!(inv
        .verify(Some("2023-05-31T14:02:45.836Z".parse()?))
        .await
        .is_err());
    Ok(())
}
