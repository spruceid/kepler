use crate::{capabilities::store::decode_root, ipfs::Block};
use anyhow::Result;
use chrono::{DateTime, Utc};
use kepler_lib::{
    didkit::DID_METHODS,
    resource::ResourceId,
    ssi::vc::{Proof, URI},
    zcap::{EncodingError, HeaderEncode, KeplerDelegation, KeplerInvocation},
};
use libipld::{
    cbor::DagCborCodec,
    codec::{Decode, Encode},
    error::Error as IpldError,
    json::DagJsonCodec,
    Cid, Ipld,
};
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use serde_json::Value;
use std::{
    convert::{TryFrom, TryInto},
    io::{Cursor, Read, Seek, Write},
    str::FromStr,
};

#[derive(Debug, Clone)]
pub struct Delegation {
    pub resources: Vec<ResourceId>,
    pub delegator: String,
    pub delegate: String,
    pub parents: Vec<Cid>,
    pub delegation: KeplerDelegation,
}

#[derive(Debug, Clone)]
pub struct Invocation {
    pub resource: ResourceId,
    pub invoker: String,
    pub parents: Vec<Cid>,
    pub invocation: KeplerInvocation,
}

#[derive(Debug, Clone)]
pub struct Revocation {
    pub parents: Vec<Cid>,
    pub revoked: Vec<Cid>,
    pub revoker: String,
    // pub revocation: KeplerInvocation,
}

#[derive(thiserror::Error, Debug)]
pub enum DelegationError {
    #[error("Invalid Resource")]
    InvalidResource,
    #[error("Missing Delegator")]
    MissingDelegator,
    #[error("Missing Delegate")]
    MissingDelegate,
}

impl TryFrom<KeplerDelegation> for Delegation {
    type Error = DelegationError;
    fn try_from(d: KeplerDelegation) -> Result<Self, Self::Error> {
        Ok(match d {
            KeplerDelegation::Ucan(u) => Self {
                resources: u
                    .payload
                    .attenuation
                    .iter()
                    .map(ResourceId::try_from)
                    .collect()?,
                delegator: u.payload.issuer,
                delegate: u.payload.audience,
                delegation: d,
                parents: u.payload.proof.clone(),
            },
            KeplerDelegation::Cacao(c) => Self {
                resources: c
                    .payload()
                    .resources
                    .iter()
                    .map(|u| ResourceId::from_str(&u))
                    .collect()?,
                delegator: c.payload().iss.to_string(),
                delegate: c.payload().aud.to_string(),
                delegation: d,
                parents: Vec::new(),
            },
        })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum InvocationError {
    #[error("Missing Resource")]
    MissingResource,
}

impl TryFrom<KeplerInvocation> for Invocation {
    type Error = InvocationError;
    fn try_from(i: KeplerInvocation) -> Result<Self, Self::Error> {
        Ok(Self {
            resource: i
                .payload
                .attenuation
                .iter()
                .find_map(|c| ResourceId::try_from(*c).ok())
                .ok_or(InvocationError::MissingResource)?,
            invoker: i.payload.issuer.clone(),
            parents: i.payload.proof.clone(),
            invocation: i,
        })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum RevocationError {
    #[error("Invalid Target")]
    InvalidTarget,
    #[error("Missing Revoker")]
    MissingRevoker,
}

impl TryFrom<KeplerInvocation> for Revocation {
    type Error = RevocationError;
    fn try_from(i: KeplerInvocation) -> Result<Self, Self::Error> {
        todo!()
        // match (
        //     i.property_set.invocation_target.path(),
        //     i.proof
        //         .as_ref()
        //         .and_then(|p| p.verification_method.as_ref()),
        // ) {
        //     (None, _) => Err(RevocationError::InvalidTarget),
        //     (_, None) => Err(RevocationError::MissingRevoker),
        //     _ => Ok(Self(i)),
        // }
    }
}

impl TryFrom<Invocation> for Revocation {
    type Error = RevocationError;
    fn try_from(i: Invocation) -> Result<Self, Self::Error> {
        i.0.try_into()
    }
}

// macro_rules! impl_fromreq {
//     ($type:ident, $name:tt) => {
//         #[rocket::async_trait]
//         impl<'r> FromRequest<'r> for $type {
//             type Error = EncodingError;
//             async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
//                 match request.headers().get_one($name).map($type::decode) {
//                     Some(Ok(item)) => Outcome::Success(item),
//                     Some(Err(e)) => Outcome::Failure((Status::Unauthorized, e)),
//                     None => Outcome::Forward(()),
//                 }
//             }
//         }
//     };
// }

// impl_fromreq!(KeplerDelegation, "Authorization");
// impl_fromreq!(KeplerInvocation, "Authorization");

pub trait CapNode {
    fn id(&self) -> &Cid;
    fn parent_ids(&self) -> NestedIdIter;
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
            fn id(&self) -> &Cid {
                &self.cid
            }
            fn parent_ids(&self) -> NestedIdIter {
                NestedIdIter(self.parents.iter())
            }
        }
    };
}

impl_capnode!(Delegation);
impl_capnode!(Invocation);
impl_capnode!(Revocation);

pub struct NestedIdIter<'a>(std::slice::Iter<'a, Cid>);

impl<'a> Iterator for NestedIdIter<'a> {
    type Item = &'a Cid;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
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
impl Verifiable for KeplerDelegation {
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
            if p.delegate != self.invoker {
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
        } else if compare_root_with_issuer(self.root(), &self.invoker).is_err() {
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    async fn basic() -> Result<()> {
        let inv_str = r#"{
          "@context": "https://w3id.org/security/v2",
          "id": "urn:uuid:689d5b45-3852-4587-9631-6f806659b16a",
          "invocationTarget": "kepler:pkh:eip155:1:0xFa4b15f717c463DF4952c59B1647169E0a5A6A78://default/kv/plaintext#get",
          "proof": {
            "type": "Ed25519Signature2018",
            "proofPurpose": "capabilityInvocation",
            "verificationMethod": "did:key:z6MkeyRTDGHqMCnq5GBZ5HnWBUy4S8B2vun1hDLjX3qyrddG#z6MkeyRTDGHqMCnq5GBZ5HnWBUy4S8B2vun1hDLjX3qyrddG",
            "created": "2022-05-31T18:01:50.391Z",
            "jws": "eyJhbGciOiJFZERTQSIsImNyaXQiOlsiYjY0Il0sImI2NCI6ZmFsc2V9..LqkVDoIzkKeB0aUv5aOtP8Jpj_fpeZya3fsBUAQzph81Io_wQ1iSb-a1l1OEu6nfLF5qQX63MrSfjYwK6HUrDQ",
            "capabilityChain": [
              "urn:zcap:root:kepler%3Apkh%3Aeip155%3A1%3A0xFa4b15f717c463DF4952c59B1647169E0a5A6A78%3A%2F%2Fdefault",
              {
                "@context": [
                  "https://w3id.org/security/v2",
                  "https://demo.didkit.dev/2022/cacao-zcap/contexts/v1.json"
                ],
                "allowedAction": [
                  "put",
                  "get",
                  "list",
                  "del",
                  "metadata"
                ],
                "cacaoPayloadType": "eip4361",
                "cacaoZcapSubstatement": "Allow access to your Kepler orbit using this session key.",
                "expires": "2022-05-31T19:01:49.766Z",
                "id": "urn:uuid:8fef6410-b8f3-401e-89ba-93878cd36c11",
                "invocationTarget": "kepler:pkh:eip155:1:0xFa4b15f717c463DF4952c59B1647169E0a5A6A78://default/kv",
                "invoker": "did:key:z6MkeyRTDGHqMCnq5GBZ5HnWBUy4S8B2vun1hDLjX3qyrddG#z6MkeyRTDGHqMCnq5GBZ5HnWBUy4S8B2vun1hDLjX3qyrddG",
                "parentCapability": "urn:zcap:root:kepler%3Apkh%3Aeip155%3A1%3A0xFa4b15f717c463DF4952c59B1647169E0a5A6A78%3A%2F%2Fdefault",
                "proof": {
                  "cacaoSignatureType": "eip191",
                  "capabilityChain": [
                    "urn:zcap:root:kepler%3Apkh%3Aeip155%3A1%3A0xFa4b15f717c463DF4952c59B1647169E0a5A6A78%3A%2F%2Fdefault"
                  ],
                  "created": "2022-05-31T18:01:49.766Z",
                  "domain": "example.com",
                  "nonce": "NJWIlpPuAXh",
                  "proofPurpose": "capabilityDelegation",
                  "proofValue": "f149ea9e2b6b4e804552c7b18d596461ed886e79fa187525149e422ac1f5ec16d0849f780107362169f4b0f51fc9b71cbd035d6f101f706c1a28c0cea07432fd71c",
                  "type": "CacaoZcapProof2022",
                  "verificationMethod": "did:pkh:eip155:1:0xFa4b15f717c463DF4952c59B1647169E0a5A6A78#blockchainAccountId"
                },
                "type": "CacaoZcap2022"
              }
            ]
          }
        }"#;
        let inv: Invocation =
            serde_json::from_str(inv_str).expect("failed to deserialize invocation");
        inv.verify(Some("2022-05-31T14:02:45.836Z".parse()?))
            .await
            .expect("failed to verify invocation");
        assert!(inv
            .verify(Some("2023-05-31T14:02:45.836Z".parse()?))
            .await
            .is_err());
        Ok(())
    }
}
