use crate::{
    capabilities::store::{FromBlock, ToBlock},
    ipfs::Block,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use kepler_lib::libipld::Cid;
use kepler_lib::{
    didkit::DID_METHODS,
    resource::{KRIParseError, ResourceId},
    zcap::{EncodingError, HeaderEncode, KeplerDelegation, KeplerInvocation, KeplerRevocation},
};
use rocket::{
    futures::future::try_join_all,
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use std::{convert::TryFrom, str::FromStr};

#[derive(Debug, Clone)]
pub struct Delegation {
    pub resources: Vec<ResourceId>,
    pub delegator: String,
    pub delegate: String,
    pub parents: Vec<Cid>,
    pub delegation: KeplerDelegation,
}

impl ToBlock for Delegation {
    fn to_block(&self) -> Result<Block> {
        self.delegation.to_block()
    }
}

impl FromBlock for Delegation {
    fn from_block(block: &Block) -> Result<Self> {
        Ok(KeplerDelegation::from_block(block)?.try_into()?)
    }
}

// deser -> sigs & time -> extract semantics -> verify semantics -> store

#[derive(Debug, Clone)]
pub struct Invocation {
    pub resource: ResourceId,
    pub invoker: String,
    pub parents: Vec<Cid>,
    pub invocation: KeplerInvocation,
}

impl ToBlock for Invocation {
    fn to_block(&self) -> Result<Block> {
        self.invocation
            .to_block(libipld::multihash::Code::Blake3_256)
    }
}

impl FromBlock for Invocation {
    fn from_block(block: &Block) -> Result<Self> {
        Ok(KeplerInvocation::from_block(block)?.try_into()?)
    }
}

#[derive(Debug, Clone)]
pub struct Revocation {
    pub parents: Vec<Cid>,
    pub revoked: Cid,
    pub revoker: String,
    pub revocation: KeplerRevocation,
}

impl ToBlock for Revocation {
    fn to_block(&self) -> Result<Block> {
        self.revocation.to_block()
    }
}

impl FromBlock for Revocation {
    fn from_block(block: &Block) -> Result<Self> {
        Ok(KeplerRevocation::from_block(block)?.try_into()?)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum DelegationError {
    #[error("Invalid Resource")]
    InvalidResource(#[from] KRIParseError),
    #[error("Missing Delegator")]
    MissingDelegator,
    #[error("Missing Delegate")]
    MissingDelegate,
}

impl TryFrom<KeplerDelegation> for Delegation {
    type Error = DelegationError;
    fn try_from(d: KeplerDelegation) -> Result<Self, Self::Error> {
        Ok(match d {
            KeplerDelegation::Ucan(ref u) => Self {
                resources: u
                    .payload
                    .attenuation
                    .iter()
                    .map(ResourceId::try_from)
                    .collect::<Result<Vec<ResourceId>, KRIParseError>>()?,
                delegator: u.payload.issuer.clone(),
                delegate: u.payload.audience.clone(),
                parents: u.payload.proof.clone(),
                delegation: d,
            },
            KeplerDelegation::Cacao(ref c) => Self {
                resources: c
                    .payload()
                    .resources
                    .iter()
                    .map(|u| ResourceId::from_str(u.as_str()))
                    .collect::<Result<Vec<ResourceId>, KRIParseError>>()?,
                delegator: c.payload().iss.to_string(),
                delegate: c.payload().aud.to_string(),
                parents: Vec::new(),
                delegation: d,
            },
        })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum InvocationError {
    #[error("Missing Resource")]
    MissingResource,
    #[error("Ambiguous Action")]
    Ambiguous,
    #[error(transparent)]
    ResourceParse(#[from] KRIParseError),
}

impl TryFrom<KeplerInvocation> for Invocation {
    type Error = InvocationError;
    fn try_from(invocation: KeplerInvocation) -> Result<Self, Self::Error> {
        let mut rs = invocation.payload.attenuation.iter();
        let resource = match (rs.next().map(ResourceId::try_from), rs.next()) {
            (Some(Ok(r)), None) => r,
            (None, _) | (Some(_), Some(_)) => return Err(InvocationError::Ambiguous),
            (Some(Err(e)), None) => return Err(e.into()),
        };
        Ok(Self {
            resource,
            invoker: invocation.payload.issuer.clone(),
            parents: invocation.payload.proof.clone(),
            invocation,
        })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum RevocationError {
    #[error("Invalid Target")]
    InvalidTarget,
}

impl TryFrom<KeplerRevocation> for Revocation {
    type Error = RevocationError;
    fn try_from(r: KeplerRevocation) -> Result<Self, Self::Error> {
        match r {
            KeplerRevocation::Cacao(ref c) => match c.payload().aud.as_str().split_once(":") {
                Some(("ucan", ps)) => Ok(Self {
                    parents: Vec::new(),
                    revoked: ps.parse().map_err(|_| RevocationError::InvalidTarget)?,
                    revoker: c.payload().iss.to_string(),
                    revocation: r,
                }),
                _ => Err(RevocationError::InvalidTarget),
            },
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum FromReqErr<T> {
    #[error(transparent)]
    Encoding(#[from] EncodingError),
    #[error(transparent)]
    TryFrom(T),
}

macro_rules! impl_fromreq {
    ($type:ident, $inter:ident, $name:tt) => {
        #[rocket::async_trait]
        impl<'r> FromRequest<'r> for $type {
            type Error = FromReqErr<<$type as TryFrom<$inter>>::Error>;
            async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
                match request.headers().get_one($name).map(|e| {
                    $type::try_from(<$inter as HeaderEncode>::decode(e)?)
                        .map_err(FromReqErr::TryFrom)
                }) {
                    Some(Ok(item)) => Outcome::Success(item),
                    Some(Err(e)) => Outcome::Failure((Status::Unauthorized, e)),
                    None => Outcome::Forward(()),
                }
            }
        }
    };
}

impl_fromreq!(Delegation, KeplerDelegation, "Authorization");
impl_fromreq!(Invocation, KeplerInvocation, "Authorization");
impl_fromreq!(Revocation, KeplerRevocation, "Authorization");

#[rocket::async_trait]
pub trait CapStore {
    async fn get_cap(&self, c: &Cid) -> Result<Option<Delegation>>;
}

pub struct EmptyCollection;

#[rocket::async_trait]
impl CapStore for EmptyCollection {
    async fn get_cap(&self, _: &Cid) -> Result<Option<Delegation>> {
        Ok(None)
    }
}

pub struct MultiCollection<'a, 'b, A, B>(pub &'a A, pub &'b B)
where
    A: CapStore,
    B: CapStore;

#[rocket::async_trait]
impl<'a, 'b, A, B> CapStore for MultiCollection<'a, 'b, A, B>
where
    A: CapStore + Send + Sync,
    B: CapStore + Send + Sync,
{
    async fn get_cap(&self, c: &Cid) -> Result<Option<Delegation>> {
        if let Some(d) = self.0.get_cap(c).await? {
            return Ok(Some(d));
        } else if let Some(d) = self.1.get_cap(c).await? {
            return Ok(Some(d));
        }
        Ok(None)
    }
}

#[rocket::async_trait]
pub trait Verifiable {
    async fn verify<C>(
        &self,
        store: &C,
        timestamp: Option<DateTime<Utc>>,
        root: &str,
    ) -> Result<()>
    where
        C: CapStore + Send + Sync;
}

#[rocket::async_trait]
impl Verifiable for Delegation {
    async fn verify<C>(&self, store: &C, time: Option<DateTime<Utc>>, root: &str) -> Result<()>
    where
        C: CapStore + Send + Sync,
    {
        let t = time.unwrap_or_else(Utc::now);

        match &self.delegation {
            KeplerDelegation::Ucan(u) => {
                u.verify_signature(DID_METHODS.to_resolver()).await?;
                u.payload
                    .validate_time(Some(t.timestamp_nanos() as f64 / 1e+9_f64))?;
            }
            KeplerDelegation::Cacao(c) => {
                c.verify().await?;
                if !c.payload().valid_at(&t) {
                    return Err(anyhow!("Delegation invalid at current time"));
                };
            }
        };

        if self.parents.is_empty() && self.delegator.starts_with(root) {
            // if it's a root cap without parents
            Ok(())
        } else {
            // verify parents and get delegated capabilities
            let res: Vec<ResourceId> = try_join_all(self.parents.iter().map(|c| async {
                let parent = store
                    .get_cap(c)
                    .await?
                    .ok_or_else(|| anyhow!("Cant find Parent"))?;
                if parent.delegate != self.delegator {
                    Err(anyhow!("Invalid Issuer"))
                } else {
                    parent.verify(store, Some(t), root).await?;
                    Ok(parent.resources)
                }
            }))
            .await?
            .into_iter()
            .flatten()
            .collect();

            // check capabilities are supported by parents
            if !self
                .resources
                .iter()
                .all(|r| res.iter().any(|c| r.extends(c).is_ok()))
            {
                Err(anyhow!("Capabilities Not Delegated"))
            } else {
                Ok(())
            }
        }
    }
}

#[rocket::async_trait]
impl Verifiable for Invocation {
    async fn verify<C>(&self, store: &C, time: Option<DateTime<Utc>>, root: &str) -> Result<()>
    where
        C: CapStore + Sync + Send,
    {
        let t = time.unwrap_or_else(Utc::now);

        self.invocation
            .verify_signature(DID_METHODS.to_resolver())
            .await?;
        self.invocation
            .payload
            .validate_time(Some(t.timestamp_nanos() as f64 / 1e+9_f64))?;

        if self.parents.is_empty() && self.invoker.starts_with(root) {
            // if it's a root cap without parents
            Ok(())
        } else {
            // verify parents and get delegated capabilities
            let res = try_join_all(self.parents.iter().map(|c| async {
                let parent = store
                    .get_cap(c)
                    .await?
                    .ok_or_else(|| anyhow!("Cant Find Parent"))?;
                if parent.delegate != self.invoker {
                    Err(anyhow!("Invalid Issuer"))
                } else {
                    parent.verify(store, Some(t), root).await?;
                    Ok(parent
                        .resources
                        .iter()
                        .any(|c| self.resource.extends(c).is_ok()))
                }
            }))
            .await?;

            // check capabilities are supported by parents
            if !res.iter().any(|c| *c) {
                Err(anyhow!("Capabilities Not Delegated"))
            } else {
                Ok(())
            }
        }
    }
}

#[rocket::async_trait]
impl Verifiable for Revocation {
    async fn verify<C>(&self, store: &C, time: Option<DateTime<Utc>>, root: &str) -> Result<()>
    where
        C: CapStore + Sync + Send,
    {
        todo!()
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
