use crate::{
    capabilities::store::{FromBlock, ToBlock},
    ipfs::Block,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use kepler_lib::libipld::Cid;
use kepler_lib::{
    authorization::{
        EncodingError, HeaderEncode, KeplerDelegation, KeplerInvocation, KeplerRevocation,
    },
    resolver::DID_METHODS,
    resource::{KRIParseError, ResourceId},
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
            KeplerRevocation::Cacao(ref c) => match c.payload().aud.as_str().split_once(':') {
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
        let t = time.unwrap_or_else(Utc::now);

        match &self.revocation {
            KeplerRevocation::Cacao(c) => {
                c.verify().await?;
                if !c.payload().valid_at(&t) {
                    return Err(anyhow!("Revocation invalid at current time"));
                };
            }
        };

        if self.parents.is_empty() && self.revoker.starts_with(root) {
            // if it's a root cap without parents
            Ok(())
        } else {
            // verify parents and get delegated capabilities
            let _res: Vec<ResourceId> = try_join_all(self.parents.iter().map(|c| async {
                let parent = store
                    .get_cap(c)
                    .await?
                    .ok_or_else(|| anyhow!("Cant find Parent"))?;
                if parent.delegate != self.revoker {
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

            Ok(())
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use kepler_lib::{
        resolver::DID_METHODS,
        ssi::{
            did::{Document, Source},
            did_resolve::DIDResolver,
            jwk::{Algorithm, JWK},
            jws::Header,
            ucan::{Capability, Payload, Ucan},
            vc::NumericDate,
        },
    };

    async fn gen(
        iss: &JWK,
        aud: String,
        caps: Vec<Capability>,
        exp: f64,
        prf: Vec<Cid>,
    ) -> (Document, Thing) {
        let did = DID_METHODS
            .generate(&Source::KeyAndPattern(iss, "key"))
            .unwrap();
        (
            DID_METHODS
                .resolve(&did, &Default::default())
                .await
                .1
                .unwrap(),
            gen_ucan((iss, did), aud, caps, exp, prf).await,
        )
    }
    async fn gen_ucan(
        iss: (&JWK, String),
        audience: String,
        attenuation: Vec<Capability>,
        exp: f64,
        proof: Vec<Cid>,
    ) -> Thing {
        let p = Payload {
            issuer: iss.1,
            audience,
            attenuation,
            proof,
            nonce: None,
            not_before: None,
            facts: None,
            expiration: NumericDate::try_from_seconds(exp).unwrap(),
        }
        .sign(Algorithm::EdDSA, iss.0)
        .unwrap();
        Thing {
            token: p.encode().unwrap(),
            payload: p.payload,
            header: p.header,
        }
    }

    #[derive(serde::Serialize)]
    struct Thing {
        pub token: String,
        pub payload: Payload,
        pub header: Header,
    }
    #[test]
    async fn basic() -> Result<()> {
        Ok(())
    }
}
