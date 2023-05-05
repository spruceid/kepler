use crate::{
    capabilities::store::{FromBlock, ToBlock},
    Block,
};
use anyhow::Result;
use kepler_lib::{
    authorization::{
        EncodingError, HeaderEncode, KeplerDelegation, KeplerInvocation, KeplerRevocation,
    },
    cacaos::siwe::Message,
    libipld::{multihash::Code, Cid},
    resolver::DID_METHODS,
    resource::{KRIParseError, ResourceId},
    siwe_recap::{extract_capabilities, verify_statement, Capability as SiweCap, Set},
    ssi::ucan::Capability as UcanCap,
};
use rocket::{
    futures::future::try_join_all,
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use serde::{Deserialize, Serialize};
use std::{convert::TryFrom, str::FromStr};
use time::OffsetDateTime;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Capability {
    pub resource: ResourceId,
    pub action: String,
}

#[derive(thiserror::Error, Debug)]
pub enum CapabilityCheckError {
    #[error(transparent)]
    ResourceCheck(#[from] kepler_lib::resource::ResourceCheckError),
    #[error("Invalid Action")]
    IncorrectAction,
}

impl Capability {
    fn extends(&self, base: &Capability) -> Result<(), CapabilityCheckError> {
        self.resource.extends(&base.resource)?;
        if self.action != base.action {
            Err(CapabilityCheckError::IncorrectAction)
        } else {
            Ok(())
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum CapExtractError {
    #[error(transparent)]
    ResourceParse(#[from] KRIParseError),
    #[error("Default actions are not allowed for Kepler capabilities")]
    DefaultActions,
    #[error("Invalid Extra Fields")]
    InvalidFields,
    #[error(transparent)]
    Cid(#[from] kepler_lib::libipld::cid::Error),
}

fn extract_ucan_cap<T>(c: &UcanCap<T>) -> Result<Capability, CapExtractError> {
    Ok(Capability {
        resource: c.with.to_string().parse()?,
        action: c.can.capability.clone(),
    })
}

fn extract_siwe_cap(c: SiweCap) -> Result<(Vec<Capability>, Vec<Cid>), CapExtractError> {
    if !c.default_actions.as_ref().is_empty() {
        Err(CapExtractError::DefaultActions)
    } else {
        Ok((
            c.targeted_actions
                .into_iter()
                .map(|(r, acs)| Ok((r.parse()?, acs)))
                .collect::<Result<Vec<(ResourceId, Set<String>)>, KRIParseError>>()?
                .into_iter()
                .flat_map(|(r, acs)| {
                    acs.into_iter()
                        .map(|action| Capability {
                            resource: r.clone(),
                            action,
                        })
                        .collect::<Vec<Capability>>()
                })
                .collect(),
            match &c
                .extra_fields
                .iter()
                .map(|(n, a)| (n.as_str(), a))
                .collect::<Vec<(&str, &serde_json::Value)>>()[..]
            {
                [] => vec![],
                [("parents", serde_json::Value::Array(a))] => a
                    .iter()
                    .map(|s| {
                        s.as_str()
                            .map(Cid::from_str)
                            .ok_or(kepler_lib::libipld::cid::Error::ParsingError)?
                    })
                    .collect::<Result<Vec<Cid>, kepler_lib::libipld::cid::Error>>()?,
                _ => return Err(CapExtractError::InvalidFields),
            },
        ))
    }
}

#[derive(Debug, Clone)]
pub struct Delegation {
    pub capabilities: Vec<Capability>,
    pub delegator: String,
    pub delegate: String,
    pub parents: Vec<Cid>,
    pub(crate) delegation: KeplerDelegation,
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
    pub capability: Capability,
    pub invoker: String,
    pub parents: Vec<Cid>,
    invocation: KeplerInvocation,
}

impl ToBlock for Invocation {
    fn to_block(&self) -> Result<Block> {
        self.invocation.to_block(Code::Blake3_256)
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
    revocation: KeplerRevocation,
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
    #[error(transparent)]
    InvalidCapability(#[from] CapExtractError),
    #[error("Missing Delegator")]
    MissingDelegator,
    #[error("Missing Delegate")]
    MissingDelegate,
    #[error(transparent)]
    SiweConversion(#[from] kepler_lib::cacaos::siwe_cacao::SIWEPayloadConversionError),
    #[error(transparent)]
    SiweCapError(#[from] kepler_lib::siwe_recap::Error),
    #[error("Invalid Siwe Statement")]
    InvalidStatement,
}

impl TryFrom<KeplerDelegation> for Delegation {
    type Error = DelegationError;
    fn try_from(d: KeplerDelegation) -> Result<Self, Self::Error> {
        Ok(match d {
            KeplerDelegation::Ucan(ref u) => Self {
                capabilities: u
                    .payload
                    .attenuation
                    .iter()
                    .map(extract_ucan_cap)
                    .collect::<Result<Vec<Capability>, CapExtractError>>()?,
                delegator: u.payload.issuer.clone(),
                delegate: u.payload.audience.clone(),
                parents: u.payload.proof.clone(),
                delegation: d,
            },
            KeplerDelegation::Cacao(ref c) => {
                let m: Message = c.payload().clone().try_into()?;
                if !verify_statement(&m)? {
                    return Err(DelegationError::InvalidStatement);
                };
                let (capabilities, parents) = extract_capabilities(&m)?
                    .remove(&"kepler".parse()?)
                    .map(extract_siwe_cap)
                    .transpose()?
                    .unwrap_or_default();
                Self {
                    capabilities,
                    delegator: c.payload().iss.to_string(),
                    delegate: c.payload().aud.to_string(),
                    parents,
                    delegation: d,
                }
            }
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
    ResourceParse(#[from] CapExtractError),
}

impl TryFrom<KeplerInvocation> for Invocation {
    type Error = InvocationError;
    fn try_from(invocation: KeplerInvocation) -> Result<Self, Self::Error> {
        let mut rs = invocation.payload.attenuation.iter();
        let capability = match (rs.next().map(extract_ucan_cap), rs.next()) {
            (Some(Ok(r)), None) => r,
            (None, _) | (Some(_), Some(_)) => return Err(InvocationError::Ambiguous),
            (Some(Err(e)), None) => return Err(e.into()),
        };
        Ok(Self {
            capability,
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
        timestamp: Option<OffsetDateTime>,
        root: &str,
    ) -> Result<()>
    where
        C: CapStore + Send + Sync;
}

#[rocket::async_trait]
impl Verifiable for Delegation {
    async fn verify<C>(&self, store: &C, time: Option<OffsetDateTime>, root: &str) -> Result<()>
    where
        C: CapStore + Send + Sync,
    {
        let t = time.unwrap_or_else(OffsetDateTime::now_utc);

        match &self.delegation {
            KeplerDelegation::Ucan(u) => {
                u.verify_signature(DID_METHODS.to_resolver()).await?;
                u.payload
                    .validate_time(Some(t.nanosecond() as f64 / 1e+9_f64))?;
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
            let res: Vec<Capability> = try_join_all(self.parents.iter().map(|c| async {
                let parent = store
                    .get_cap(c)
                    .await?
                    .ok_or_else(|| anyhow!("Cant find Parent"))?;
                if parent.delegate != self.delegator {
                    Err(anyhow!("Invalid Issuer"))
                } else {
                    parent.verify(store, Some(t), root).await?;
                    Ok(parent.capabilities)
                }
            }))
            .await?
            .into_iter()
            .flatten()
            .collect();

            // check capabilities are supported by parents
            if !self
                .capabilities
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
    async fn verify<C>(&self, store: &C, time: Option<OffsetDateTime>, root: &str) -> Result<()>
    where
        C: CapStore + Sync + Send,
    {
        let t = time.unwrap_or_else(OffsetDateTime::now_utc);

        self.invocation
            .verify_signature(DID_METHODS.to_resolver())
            .await?;
        self.invocation
            .payload
            .validate_time(Some(t.nanosecond() as f64 / 1e+9_f64))?;

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
                        .capabilities
                        .iter()
                        .any(|c| self.capability.extends(c).is_ok()))
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
    async fn verify<C>(&self, store: &C, time: Option<OffsetDateTime>, root: &str) -> Result<()>
    where
        C: CapStore + Sync + Send,
    {
        let t = time.unwrap_or_else(OffsetDateTime::now_utc);

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
            try_join_all(self.parents.iter().map(|c| async {
                let parent = store
                    .get_cap(c)
                    .await?
                    .ok_or_else(|| anyhow!("Cant find Parent"))?;
                if parent.delegate != self.revoker {
                    Err(anyhow!("Invalid Issuer"))
                } else {
                    parent.verify(store, Some(t), root).await?;
                    Ok(())
                }
            }))
            .await?;

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
            ucan::{Capability, Payload},
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
