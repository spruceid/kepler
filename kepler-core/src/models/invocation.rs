use super::super::{
    events::{Invocation, Operation},
    models::*,
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

    pub seq: i64,
    pub epoch_id: Hash,
    pub epoch_seq: i64,

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
        belongs_to = "epoch::Entity",
        from = "Column::EpochId",
        to = "epoch::Column::Id"
    )]
    Epoch,
}

impl Related<actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invoker.def()
    }
}

impl Related<epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
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
    root: &str,
    orbit: &str,
    db: &C,
    invocation: Invocation,
    seq: i64,
    epoch: Hash,
    epoch_seq: i64,
) -> Result<Hash, Error> {
    let Invocation(i, serialized, parameters) = invocation;
    verify(&i.invocation).await?;

    let now = OffsetDateTime::now_utc();
    validate(db, root, orbit, &i, Some(now)).await?;

    save(
        db,
        orbit,
        i,
        Some(now),
        serialized,
        (seq, epoch, epoch_seq),
        parameters,
    )
    .await
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
    root: &str,
    orbit: &str,
    invocation: &util::InvocationInfo,
    time: Option<OffsetDateTime>,
) -> Result<(), Error> {
    if !invocation.parents.is_empty() && !invocation.invoker.starts_with(root) {
        let parents = delegation::Entity::find()
            .filter(delegation::Column::Orbit.eq(orbit))
            .filter(invocation.parents.iter().fold(Condition::any(), |cond, p| {
                cond.add(delegation::Column::Id.eq(Hash::from(*p)))
            }))
            .all(db)
            .await?;

        if parents.len() != invocation.parents.len() {
            return Err(InvocationError::MissingParents)?;
        };

        let mut parent_abilities = Vec::new();
        let now = time.unwrap_or_else(OffsetDateTime::now_utc);
        for parent in parents {
            // check parent's delegatee is invoker
            if parent.delegatee != invocation.invoker {
                return Err(InvocationError::UnauthorizedInvoker(
                    invocation.invoker.clone(),
                ))?;
            };
            // check expiry of parent
            if parent.expiry < Some(now) {
                return Err(InvocationError::InvalidTime)?;
            };
            // check nbf of parent
            if parent.not_before.map(|pnbf| pnbf > now).unwrap_or(false) {
                return Err(InvocationError::InvalidTime)?;
            };
            // TODO check revocation status of parents
            parent_abilities.extend(parent.find_related(abilities::Entity).all(db).await?);
        }
        if !parent_abilities.iter().any(|pab| {
            invocation.capability.resource.starts_with(&pab.resource)
                && invocation.capability.action == pab.ability
        }) {
            return Err(InvocationError::UnauthorizedCapability(
                invocation.capability.resource.clone(),
                invocation.capability.action.clone(),
            ))?;
        }
    } else if !invocation.invoker.starts_with(root) {
        return Err(InvocationError::UnauthorizedInvoker(
            invocation.invoker.clone(),
        ))?;
    }

    Ok(())
}

async fn save<C: ConnectionTrait>(
    db: &C,
    orbit: &str,
    invocation: util::InvocationInfo,
    time: Option<OffsetDateTime>,
    serialization: Vec<u8>,
    event_version: (i64, Hash, i64),
    parameters: Option<Operation>,
) -> Result<Hash, Error> {
    let hash = crate::hash::hash(&serialization);
    let issued_at = time.unwrap_or_else(OffsetDateTime::now_utc);

    Entity::insert(ActiveModel::from(Model {
        seq: event_version.0,
        epoch_id: event_version.1,
        epoch_seq: event_version.2,
        id: hash,
        issued_at,
        serialization,
        invoker: invocation.invoker,
        resource: invocation.capability.resource,
        ability: invocation.capability.action,
        orbit: orbit.to_string(),
    }))
    .exec(db)
    .await?;

    match parameters {
        Some(Operation::KvWrite {
            key,
            value,
            metadata,
        }) => {
            kv_write::Entity::insert(kv_write::ActiveModel::from(kv_write::Model {
                key,
                value,
                seq: event_version.0,
                epoch_id: event_version.1,
                invocation_id: hash,
                orbit: orbit.to_string(),
                metadata,
            }))
            .exec(db)
            .await?;
        }
        Some(Operation::KvDelete { key, version }) => {
            let (deleted_seq, deleted_epoch_id) = match version {
                Some((seq, epoch_id)) => (seq, epoch_id),
                None => {
                    let kv = kv_write::Entity::find()
                        .filter(kv_write::Column::Key.eq(key.clone()))
                        .order_by_desc(kv_write::Column::Seq)
                        .order_by_desc(kv_write::Column::EpochId)
                        .one(db)
                        .await?
                        .ok_or_else(|| InvocationError::MissingKvWrite(key.clone()))?;
                    (kv.seq, kv.epoch_id)
                }
            };
            kv_delete::Entity::insert(kv_delete::ActiveModel::from(kv_delete::Model {
                key,
                seq: event_version.0,
                epoch_id: event_version.1,
                invocation_id: hash,
                deleted_seq,
                deleted_epoch_id,
                orbit: orbit.to_string(),
            }))
            .exec(db)
            .await?;
        }
        None => {}
    };

    Ok(hash)
}
