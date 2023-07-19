use super::super::{
    events::{Invocation, VersionedOperation},
    models::*,
    relationships::*,
    util,
};
use crate::hash::Hash;
use crate::types::{Facts, OrbitIdWrap, Resource};
use kepler_lib::{authorization::KeplerInvocation, resolver::DID_METHODS};
use sea_orm::{entity::prelude::*, sea_query::OnConflict, Condition, ConnectionTrait, QueryOrder};
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "invocation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, unique)]
    pub id: Hash,

    pub invoker: String,
    pub issued_at: OffsetDateTime,
    pub facts: Option<Facts>,
    pub serialization: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // inverse relation, invocations belong to invokers
    #[sea_orm(
        belongs_to = "actor::Entity",
        from = "Column::Invoker",
        to = "actor::Column::Id"
    )]
    Invoker,
    #[sea_orm(has_many = "invoked_abilities::Entity")]
    InvokedAbilities,
}

impl Related<actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invoker.def()
    }
}

impl Related<invoked_abilities::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::InvokedAbilities.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Db(#[from] DbErr),
    #[error(transparent)]
    InvalidInvocation(#[from] InvocationError),
}

#[derive(Debug, thiserror::Error)]
pub enum InvocationError {
    #[error("Invocation expired or not yet valid")]
    InvalidTime,
    #[error("Failed to verify signature")]
    InvalidSignature,
    #[error("Unauthorized Invoker")]
    UnauthorizedInvoker(String),
    #[error("Unauthorized Action: {0} / {1}")]
    UnauthorizedAction(Resource, String),
    #[error("Cannot find parent delegation")]
    MissingParents,
    #[error("No Such Key: {0}")]
    MissingKvWrite(String),
}

pub(crate) async fn process<C: ConnectionTrait>(
    db: &C,
    invocation: Invocation,
    ops: Vec<VersionedOperation>,
) -> Result<Hash, Error> {
    let (i, serialized) = (invocation.0, invocation.1);
    verify(&i.invocation).await?;

    let now = OffsetDateTime::now_utc();
    validate(db, &i, Some(now)).await?;

    save(db, i, Some(now), serialized, ops).await
}

async fn verify(invocation: &KeplerInvocation) -> Result<(), Error> {
    invocation
        .verify_signature(DID_METHODS.to_resolver())
        .await
        .map_err(|_| InvocationError::InvalidSignature)?;
    invocation
        .payload
        .validate_time(None)
        .map_err(|_| InvocationError::InvalidTime)?;
    Ok(())
}

// verify parenthood and authorization
async fn validate<C: ConnectionTrait>(
    db: &C,
    invocation: &util::InvocationInfo,
    time: Option<OffsetDateTime>,
) -> Result<(), Error> {
    // get caps which rely on delegated caps
    let dependant_caps: Vec<_> = invocation
        .capabilities
        .iter()
        .filter(|c| {
            // remove caps for which the invoker is the root authority
            c.resource
                .orbit()
                .map(|o| o.did() != invocation.invoker)
                .unwrap_or(true)
        })
        .collect();

    match (dependant_caps.is_empty(), invocation.parents.is_empty()) {
        // no dependant caps, no parents needed, must be valid
        (true, _) => Ok(()),
        // dependant caps, no parents, invalid
        (false, true) => Err(InvocationError::MissingParents.into()),
        // dependant caps, parents, check parents
        (false, false) => {
            // get parents which have
            let parents = delegation::Entity::find()
                // the correct id
                .filter(
                    delegation::Column::Id.is_in(invocation.parents.iter().map(|c| Hash::from(*c))),
                )
                // and also get their abilities
                .find_with_related(abilities::Entity)
                .all(db)
                .await?;

            // check parent identifies correct invoker
            parents
                .iter()
                .map(|(p, _)| {
                    if p.delegatee != invocation.invoker
                        && !invocation.invoker.starts_with(&p.delegatee)
                    {
                        Err(InvocationError::UnauthorizedInvoker(
                            invocation.invoker.clone(),
                        ))
                    } else {
                        Ok(())
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;

            let now = time.unwrap_or_else(OffsetDateTime::now_utc);

            // only use parents which are valid at the time of invocation
            let parents: Vec<_> = parents
                .into_iter()
                .filter(|(p, _)| {
                    p.expiry.map(|pexp| now < pexp).unwrap_or(true)
                        && p.not_before.map(|pnbf| now >= pnbf).unwrap_or(true)
                })
                .collect();

            // check each dependant cap is supported by at least one parent cap
            match dependant_caps.iter().find(|c| {
                !parents
                    .iter()
                    .flat_map(|(_, a)| a)
                    .any(|pc| c.resource.extends(&pc.resource) && c.action == pc.ability)
            }) {
                Some(c) => Err(InvocationError::UnauthorizedAction(
                    c.resource.clone(),
                    c.action.clone(),
                )
                .into()),
                None => Ok(()),
            }
        }
    }
}

async fn save<C: ConnectionTrait>(
    db: &C,
    invocation: util::InvocationInfo,
    time: Option<OffsetDateTime>,
    serialization: Vec<u8>,
    parameters: Vec<VersionedOperation>,
) -> Result<Hash, Error> {
    let hash = crate::hash::hash(&serialization);
    let issued_at = time.unwrap_or_else(OffsetDateTime::now_utc);

    match Entity::insert(ActiveModel::from(Model {
        id: hash,
        issued_at,
        serialization,
        facts: None,
        invoker: invocation.invoker,
    }))
    .on_conflict(OnConflict::column(Column::Id).do_nothing().to_owned())
    .exec(db)
    .await
    {
        Err(DbErr::RecordNotInserted) => return Ok(hash),
        r => {
            r?;
        }
    };

    // save invoked abilities
    if !invocation.capabilities.is_empty() {
        invoked_abilities::Entity::insert_many(invocation.capabilities.into_iter().map(|c| {
            invoked_abilities::ActiveModel::from(invoked_abilities::Model {
                invocation: hash,
                resource: c.resource,
                ability: c.action,
            })
        }))
        .exec(db)
        .await?;
    }
    // save parent relationships
    if !invocation.parents.is_empty() {
        parent_delegations::Entity::insert_many(invocation.parents.into_iter().map(|p| {
            parent_delegations::ActiveModel::from(parent_delegations::Model {
                child: hash,
                parent: p.into(),
            })
        }))
        .exec(db)
        .await?;
    }

    for param in parameters {
        match param {
            VersionedOperation::KvWrite {
                key,
                value,
                metadata,
                orbit,
                seq,
                epoch,
                epoch_seq,
            } => {
                kv_write::Entity::insert(kv_write::ActiveModel::from(kv_write::Model {
                    invocation: hash,
                    key,
                    value,
                    orbit: orbit.into(),
                    metadata,
                    seq,
                    epoch,
                    epoch_seq,
                }))
                .exec(db)
                .await?;
            }
            VersionedOperation::KvDelete {
                key,
                version,
                orbit,
            } => {
                let deleted_invocation_id = if let Some((s, e, es)) = version {
                    kv_write::Entity::find().filter(
                        Condition::all()
                            .add(kv_write::Column::Key.eq(key.clone()))
                            .add(kv_write::Column::Orbit.eq(OrbitIdWrap(orbit.clone())))
                            .add(kv_write::Column::Seq.eq(s))
                            .add(kv_write::Column::Epoch.eq(e))
                            .add(kv_write::Column::EpochSeq.eq(es)),
                    )
                } else {
                    kv_write::Entity::find()
                        .filter(kv_write::Column::Key.eq(key.clone()))
                        .filter(kv_write::Column::Orbit.eq(OrbitIdWrap(orbit.clone())))
                        .order_by_desc(kv_write::Column::Seq)
                        .order_by_desc(kv_write::Column::Epoch)
                        .order_by_desc(kv_write::Column::EpochSeq)
                }
                .one(db)
                .await?
                .ok_or_else(|| InvocationError::MissingKvWrite(key.clone()))?
                .invocation;
                kv_delete::Entity::insert(kv_delete::ActiveModel::from(kv_delete::Model {
                    key,
                    invocation_id: hash,
                    orbit: orbit.into(),
                    deleted_invocation_id,
                }))
                .exec(db)
                .await?;
            }
        }
    }

    Ok(hash)
}
