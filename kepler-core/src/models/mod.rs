pub mod abilities;
pub mod actor;
pub mod delegation;
pub mod epoch;
pub mod invocation;
pub mod kv_delete;
pub mod kv_write;
pub mod orbit;
pub mod revocation;

use crate::{hash::Hash, keys::Secrets, storage::StorageSetup, types::CaveatsInner, TxError};
use kepler_lib::{
    authorization::Resources,
    cacaos::{
        common::{CommonCacao, CommonVerifier, Error as CacaoError},
        Cacao,
    },
    iri_string::types::{UriStr, UriString},
    resolver::DID_METHODS,
    resource::{AnyResource, ResourceId},
    ssi::ucan::capabilities::{Ability, NotaBeneCollection},
};
use sea_orm::entity::prelude::*;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    iter::once,
};
use time::OffsetDateTime;

#[derive(Debug, thiserror::Error)]
pub enum EventProcessingError {
    #[error(transparent)]
    Db(#[from] DbErr),
    #[error(transparent)]
    InvalidMessage(#[from] ValidationError),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("Missing event for service: {0} {1} {2:?}")]
    MissingServiceEvent(ResourceId, String, Option<(i64, Hash, i64)>),
}

impl EventProcessingError {
    pub(crate) fn into_del<S: StorageSetup, K: Secrets>(self) -> TxError<S, K> {
        match self {
            EventProcessingError::Db(e) => TxError::Db(e),
            EventProcessingError::InvalidMessage(e) => TxError::InvalidDelegation(e),
            EventProcessingError::Serde(e) => TxError::Serde(e),
            EventProcessingError::MissingServiceEvent(id, service, version) => {
                TxError::MissingServiceEvent(id, service, version)
            }
        }
    }
    pub(crate) fn into_inv<S: StorageSetup, K: Secrets>(self) -> TxError<S, K> {
        match self {
            EventProcessingError::Db(e) => TxError::Db(e),
            EventProcessingError::InvalidMessage(e) => TxError::InvalidInvocation(e),
            EventProcessingError::Serde(e) => TxError::Serde(e),
            EventProcessingError::MissingServiceEvent(id, service, version) => {
                TxError::MissingServiceEvent(id, service, version)
            }
        }
    }
}

impl From<CacaoError> for EventProcessingError {
    fn from(e: CacaoError) -> Self {
        Self::InvalidMessage(e.into())
    }
}

impl From<time::error::ComponentRange> for EventProcessingError {
    fn from(e: time::error::ComponentRange) -> Self {
        Self::InvalidMessage(e.into())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Message expired or not yet valid")]
    InvalidTime,
    #[error("Failed to verify signature: {0}")]
    InvalidSignature(#[from] CacaoError),
    #[error("Unauthorized Issuer: {0}")]
    UnauthorizedIssuer(String),
    #[error("Unauthorized Capabilities: {0:?}")]
    UnauthorizedCapability(HashMap<AnyResource, HashSet<Ability>>),
    #[error("Cannot find parent delegation")]
    MissingParents,
    #[error(transparent)]
    UnixTimeError(#[from] time::error::ComponentRange),
}

// verify signature
async fn verify(cacao: &CommonCacao) -> Result<(), ValidationError> {
    Ok(cacao.verify(&CommonVerifier::new(&*DID_METHODS)).await?)
}

// verify parenthood and authorization
async fn validate<'a, C: ConnectionTrait>(
    db: &C,
    message: &'a CommonCacao,
    time: Option<OffsetDateTime>,
) -> Result<(), EventProcessingError> {
    let mut required = get_required(message);
    match (required.next(), message.proof()) {
        // no dependant caps, no parents needed, must be valid
        (None, _) => Ok(()),
        // dependant caps and parents, check parents
        (Some(rf), Some(prf)) if !prf.is_empty() => {
            let mut unauthorized = take_unauthorized(
                once(rf).chain(required),
                // get all known parents of `message`
                get_granted(db, message, time).await?,
            )
            .map(|(r, a)| (r.into(), a.into_iter().cloned().collect()));
            match unauthorized.next() {
                Some(uf) => Err(ValidationError::UnauthorizedCapability(
                    once(uf).chain(unauthorized).collect(),
                )
                .into()),
                _ => Ok(()),
            }
        }
        // dependant caps, no parents, invalid
        _ => Err(ValidationError::MissingParents.into()),
    }
}

// get caps which rely on delegated parent caps
fn get_required<'a, S: 'a, F: 'a, NB: 'a>(
    message: &'a Cacao<S, F, NB>,
) -> impl Iterator<
    Item = (
        AnyResource<&'a UriStr>,
        &'a BTreeMap<Ability, NotaBeneCollection<NB>>,
    ),
> {
    Resources::<'a, AnyResource<&'a UriStr>, NB>::grants(message)
        // remove caps for which the delegator is the root authority
        .filter(|(r, _)| {
            r.orbit().map_or(true, |o| {
                o.suffix() != message.issuer().method().to_string()
            })
        })
}

// check each actioned cap is supported by at least one granted cap
// return caps which are not supported
fn take_unauthorized<'a>(
    actioned: impl Iterator<Item = (AnyResource<&'a UriStr>, &'a BTreeMap<Ability, CaveatsInner>)>,
    granted: HashMap<AnyResource<UriString>, BTreeMap<String, CaveatsInner>>,
) -> impl Iterator<Item = (AnyResource<&'a UriStr>, HashSet<&'a Ability>)> {
    actioned.filter_map(move |(r, a)| {
        let unsupported = a
            .keys()
            .filter(|ab| {
                // get unsupported abilities
                !granted
                    .iter()
                    // only get applicable caps where the resource is right
                    .filter_map(|(gr, ga)| r.extends(gr).then_some(ga))
                    // and the ability is not supported
                    .any(|ga| ga.contains_key(ab.as_ref()))
            })
            .collect::<HashSet<_>>();
        if unsupported.is_empty() {
            None
        } else {
            Some((r, unsupported))
        }
    })
}

async fn get_granted<C: ConnectionTrait>(
    db: &C,
    message: &CommonCacao,
    time: Option<OffsetDateTime>,
) -> Result<HashMap<AnyResource, BTreeMap<String, CaveatsInner>>, EventProcessingError> {
    Ok(match message.proof() {
        // get delegated abilities from each parent
        Some(prf) if !prf.is_empty() => {
            let issuer = message.issuer().to_string();
            let nbf = message
                .not_before()
                .map(|i| OffsetDateTime::from_unix_timestamp(i as i64))
                .transpose()?;
            let exp = message
                .expiration()
                .map(|i| OffsetDateTime::from_unix_timestamp(i as i64))
                .transpose()?;

            delegation::Entity::find()
                // get parents which have
                // the correct id
                .filter(delegation::Column::Id.is_in(prf.iter().map(|c| Hash::from(*c))))
                // the correct delegatee
                .filter(delegation::Column::Delegatee.eq(&issuer))
                // unrevoked
                .left_join(revocation::Entity)
                .filter(revocation::Column::Id.is_null())
                .all(db)
                .await?
                .into_iter()
                // valid issuer
                .filter(|p| p.delegatee == issuer)
                // valid time bounds
                .filter(|p| p.validate_bounds(nbf, exp))
                // extra check
                .filter(|p| time.map_or(true, |t| p.valid_at(t, None)))
                .collect::<Vec<_>>()
                .load_many(abilities::Entity, db)
                .await?
                .into_iter()
                .flatten()
                .fold(HashMap::new(), |mut acc, pc| {
                    acc.entry(pc.resource.0)
                        .or_default()
                        .entry(pc.ability.0.to_string())
                        .or_default()
                        .extend(pc.caveats.0);
                    acc
                })
        }
        _ => HashMap::new(),
    })
}
