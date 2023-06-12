use super::super::{
    events::{Invocation, Operation},
    models::*,
    relationships::*,
    util,
};
use crate::hash::Hash;
use kepler_lib::{authorization::KeplerInvocation, resolver::DID_METHODS};
use sea_orm::{entity::prelude::*, sea_query::Condition, ConnectionTrait, QueryOrder};
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "invocation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, unique)]
    pub id: Hash,

    pub invoker: String,
    pub issued_at: OffsetDateTime,
    pub serialization: Vec<u8>,
    pub resource: String,
    pub ability: String,
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
    // inverse relation, invocations belong to epochs
    #[sea_orm(
        belongs_to = "event_order::Entity",
        from = "Column::Id",
        to = "event_order::Column::Event"
    )]
    Ordering,
    #[sea_orm(
        belongs_to = "parent_delegations::Entity",
        from = "Column::Id",
        to = "parent_delegations::Column::Child"
    )]
    Parents,
}

impl Related<actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invoker.def()
    }
}

impl Related<event_order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Ordering.def()
    }
}

impl Related<delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Parents.def()
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
    #[error("Unauthorized Capability")]
    UnauthorizedCapability(String, String),
    #[error("Cannot find parent delegation")]
    MissingParents,
    #[error("No Such Key: {0}")]
    MissingKvWrite(String),
}

pub(crate) async fn process<C: ConnectionTrait>(
    db: &C,
    invocation: Invocation,
) -> Result<Hash, Error> {
    let Invocation(i, serialized, parameters) = invocation;
    verify(&i.invocation).await?;

    let now = OffsetDateTime::now_utc();
    validate(db, &i, Some(now)).await?;

    save(db, i, Some(now), serialized, parameters).await
}

async fn verify(invocation: &KeplerInvocation) -> Result<(), Error> {
    invocation
        .verify_signature(DID_METHODS.to_resolver())
        .await
        .map_err(|_| InvocationError::InvalidSignature)?;
    // TODO bug in kepler-sdk, it doesnt take all nanoseconds, just the offset from current second
    // invocation
    //     .payload
    //     .validate_time(None)
    //     .map_err(|_| InvocationError::InvalidTime)?;
    Ok(())
}

// verify parenthood and authorization
async fn validate<C: ConnectionTrait>(
    db: &C,
    invocation: &util::InvocationInfo,
    time: Option<OffsetDateTime>,
) -> Result<(), Error> {
    // get caps which rely on delegated caps
    let dependant_caps = invocation
        .capabilities
        .iter()
        .filter(|c| {
            // remove caps for which the delegator is the root authority
            c.resource
                .parse()
                .ok()
                .map(|r| r.did() != invocation.invoker)
                .unwrap_or(true)
        })
        .collect();

    match (dependant_caps.is_empty(), invocation.parents.is_empty()) {
        // no dependant caps, no parents needed, must be valid
        (true, _) => Ok(()),
        // dependant caps, no parents, invalid
        (false, true) => Err(InvocationError::MissingParents),
        // dependant caps, parents, check parents
        (false, false) => {
            // get parents which have
            let parents = delegation::Entity::find()
                // the correct id
                .filter(
                    delegation::Column::Id.is_in(invocation.parents.iter().map(|c| Hash::from(*c))),
                )
                // the correct delegatee
                .filter(delegation::Column::Delegatee.eq(invocation.invoker))
                .all(db)
                .await?;

            let now = time.unwrap_or_else(OffsetDateTime::now_utc);
            let parents = parents
                .into_iter()
                .filter(|p| {
                    // valid time bounds
                    p.expiry < Some(now) && p.not_before.map(|pnbf| pnbf > now).unwrap_or(false)
                })
                .collect();

            // get delegated abilities from each parent
            let parent_abilities = parents.find_many(abilities::Entity, db).await?;

            // check each dependant cap is supported by at least one parent cap
            match !dependant_caps.iter().first(|c| {
                !parent_abilities
                    .iter()
                    .any(|pc| c.resource.starts_with(&pc.resource) && c.action == pc.ability)
            }) {
                Some(c) => Err(InvocationError::UnauthorizedCapability(
                    c.resource, c.ability,
                )),
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
    parameters: Option<(Operation, Hash)>,
) -> Result<Hash, Error> {
    let hash = crate::hash::hash(&serialization);
    let issued_at = time.unwrap_or_else(OffsetDateTime::now_utc);

    Entity::insert(ActiveModel::from(Model {
        id: hash,
        issued_at,
        serialization,
        invoker: invocation.invoker,
        resource: invocation.capability.resource,
        ability: invocation.capability.action,
    }))
    .exec(db)
    .await?;

    // TODO insert invoked_abilities
    // save parent relationships
    parent_delegations::Entity::insert_many(invocation.parents.into_iter().map(|p| {
        parent_delegations::ActiveModel::from(parent_delegations::Model {
            child: hash,
            parent: p.into(),
        })
    }))
    .exec(db)
    .await?;

    match parameters {
        Some(Operation::KvWrite {
            key,
            value,
            metadata,
            orbit,
        }) => {
            kv_write::Entity::insert(kv_write::ActiveModel::from(kv_write::Model {
                invocation: hash,
                key,
                value,
                orbit: orbit.to_string(),
                metadata,
            }))
            .exec(db)
            .await?;
        }
        Some(Operation::KvDelete {
            key,
            version,
            orbit,
        }) => {
            let orbit = orbit.to_string();
            let deleted_invocation_id = match version {
                Some((seq, epoch_id)) => todo!("get invocation ID of kvwrite"),
                None => {
                    kv_write::Entity::find()
                        .filter(kv_write::Column::Key.eq(key))
                        .filter(kv_write::Column::Orbit.eq(orbit))
                        .order_by_desc(kv_write::Column::Seq)
                        .order_by_desc(kv_write::Column::EpochId)
                        .one(db)
                        .await?
                        .ok_or_else(|| InvocationError::MissingKvWrite(key.clone()))?
                        .invocation
                }
            };
            kv_delete::Entity::insert(kv_delete::ActiveModel::from(kv_delete::Model {
                key,
                invocation_id: hash,
                orbit,
                deleted_invocation_id,
            }))
            .exec(db)
            .await?;
        }
        None => {}
    };

    Ok(hash)
}
