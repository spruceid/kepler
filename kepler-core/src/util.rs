use crate::types::Resource;
use kepler_lib::{
    authorization::{KeplerDelegation, KeplerInvocation, KeplerRevocation},
    cacaos::siwe::Message,
    libipld::Cid,
    resource::OrbitId,
    siwe_recap::{extract_capabilities, verify_statement, Capability as SiweCap},
    ssi::ucan::Capability as UcanCap,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use time::OffsetDateTime;

#[derive(Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq)]
pub struct Capability {
    pub resource: Resource,
    pub action: String,
}

#[non_exhaustive]
#[derive(thiserror::Error, Debug)]
pub enum CapExtractError {
    #[error("Default actions are not allowed for Kepler capabilities")]
    DefaultActions,
    #[error("Invalid Extra Fields")]
    InvalidFields,
    #[error(transparent)]
    Cid(#[from] kepler_lib::libipld::cid::Error),
}

fn extract_ucan_cap<T>(c: &UcanCap<T>) -> Result<Capability, CapExtractError> {
    Ok(Capability {
        resource: c.with.to_string().into(),
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
                .flat_map(|(r, acs)| {
                    acs.into_iter()
                        .map(|action| Capability {
                            resource: Resource::from(r.clone()),
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
pub struct DelegationInfo {
    pub capabilities: Vec<Capability>,
    pub delegator: String,
    pub delegate: String,
    pub parents: Vec<Cid>,
    pub delegation: KeplerDelegation,
    pub expiry: Option<OffsetDateTime>,
    pub not_before: Option<OffsetDateTime>,
    pub issued_at: Option<OffsetDateTime>,
}

impl DelegationInfo {
    pub fn orbits(&self) -> impl Iterator<Item = &OrbitId> + '_ {
        self.capabilities.iter().filter_map(|c| c.resource.orbit())
    }
}

#[non_exhaustive]
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

impl TryFrom<KeplerDelegation> for DelegationInfo {
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
                expiry: OffsetDateTime::from_unix_timestamp_nanos(
                    (u.payload.expiration.as_seconds() * 1_000_000_000.0) as i128,
                )
                .ok(),
                not_before: u.payload.not_before.and_then(|t| {
                    OffsetDateTime::from_unix_timestamp_nanos(
                        (t.as_seconds() * 1_000_000_000.0) as i128,
                    )
                    .ok()
                }),
                delegation: d,
                issued_at: None,
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
                    expiry: c.payload().exp.as_ref().map(|t| *t.as_ref()),
                    not_before: c.payload().nbf.as_ref().map(|t| *t.as_ref()),
                    issued_at: Some(*c.payload().iat.as_ref()),
                    delegation: d,
                }
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct InvocationInfo {
    pub capabilities: Vec<Capability>,
    pub invoker: String,
    pub parents: Vec<Cid>,
    pub invocation: KeplerInvocation,
}

impl InvocationInfo {
    pub fn orbits(&self) -> impl Iterator<Item = &OrbitId> + '_ {
        self.capabilities.iter().filter_map(|c| c.resource.orbit())
    }
}

#[non_exhaustive]
#[derive(thiserror::Error, Debug)]
pub enum InvocationError {
    #[error("Missing Resource")]
    MissingResource,
    #[error(transparent)]
    ResourceParse(#[from] CapExtractError),
}

impl TryFrom<KeplerInvocation> for InvocationInfo {
    type Error = InvocationError;
    fn try_from(invocation: KeplerInvocation) -> Result<Self, Self::Error> {
        Ok(Self {
            capabilities: invocation
                .payload
                .attenuation
                .iter()
                .map(extract_ucan_cap)
                .collect::<Result<Vec<Capability>, CapExtractError>>()?,
            invoker: invocation.payload.issuer.clone(),
            parents: invocation.payload.proof.clone(),
            invocation,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RevocationInfo {
    // TODO these should be hash
    pub parents: Vec<Cid>,
    pub revoked: Cid,
    pub revoker: String,
    pub revocation: KeplerRevocation,
}

#[derive(thiserror::Error, Debug)]
pub enum RevocationError {
    #[error("Invalid Target")]
    InvalidTarget,
}

impl TryFrom<KeplerRevocation> for RevocationInfo {
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
