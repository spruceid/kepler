use super::super::{
    events::{Invocation, Operation},
    models::*,
    util,
};
use crate::hash::Hash;
use kepler_lib::{authorization::KeplerInvocation, resolver::DID_METHODS};
use sea_orm::{entity::prelude::*, sea_query::Condition, ConnectionTrait};
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "invocation")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Vec<u8>,
    #[sea_orm(primary_key)]
    pub orbit: String,

    pub seq: u64,
    pub epoch_id: Vec<u8>,
    pub epoch_seq: u64,

    pub invoker: String,
    pub issued_at: OffsetDateTime,
    pub serialization: Vec<u8>,
    pub resource: String,
    pub ability: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // inverse relation, invocations belong to parent delegations
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "Column::Id",
        to = "delegation::Column::Id"
    )]
    Parent,
    // inverse relation, invocations belong to invokers
    #[sea_orm(
        belongs_to = "actor::Entity",
        from = "(Column::Invoker, Column::Orbit)",
        to = "(actor::Column::Id, actor::Column::Orbit)"
    )]
    Invoker,
    // inverse relation, invocations belong to epochs
    #[sea_orm(
        belongs_to = "epoch::Entity",
        from = "(Column::EpochId, Column::Orbit)",
        to = "(epoch::Column::Id, epoch::Column::Orbit)"
    )]
    Epoch,
}

impl Related<delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Parent.def()
    }
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
    #[error(transparent)]
    ParameterExtraction(#[from] util::InvocationError),
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
}

pub async fn process<C: ConnectionTrait>(
    root: &str,
    orbit: &str,
    db: &C,
    invocation: Invocation,
    seq: u64,
    epoch: Hash,
    epoch_seq: u64,
) -> Result<Hash, Error> {
    let Invocation(i, serialized, parameters) = invocation;
    verify(&i).await?;

    let i_info = util::InvocationInfo::try_from(i).map_err(InvocationError::ParameterExtraction)?;
    let now = OffsetDateTime::now_utc();
    validate(db, root, orbit, &i_info, Some(now)).await?;

    save(
        db,
        orbit,
        i_info,
        Some(now),
        serialized,
        seq,
        epoch,
        epoch_seq,
        parameters,
    )
    .await
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
    root: &str,
    orbit: &str,
    invocation: &util::InvocationInfo,
    time: Option<OffsetDateTime>,
) -> Result<(), Error> {
    let now = time.unwrap_or_else(|| OffsetDateTime::now_utc());
    if !invocation.parents.is_empty() || invocation.invoker.starts_with(root) {
        let parents = delegation::Entity::find()
            .filter(Column::Orbit.eq(orbit))
            .filter(invocation.parents.iter().fold(Condition::any(), |cond, p| {
                cond.add(Column::Id.eq(p.hash().to_bytes()))
            }))
            .all(db)
            .await?;
        if parents.len() != invocation.parents.len() {
            return Err(InvocationError::MissingParents)?;
        };

        let mut parent_abilities = Vec::new();
        for parent in parents {
            // get delegatee of parent
            let delegatee = parent
                .find_related(actor::Entity)
                .one(db)
                .await?
                .ok_or_else(|| InvocationError::MissingParents)?;
            // check parent's delegatee is invoker
            if delegatee.id != invocation.invoker {
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
    }

    Ok(())
}

async fn save<C: ConnectionTrait>(
    db: &C,
    orbit: &str,
    invocation: util::InvocationInfo,
    time: Option<OffsetDateTime>,
    serialization: Vec<u8>,
    seq: u64,
    epoch: Hash,
    epoch_seq: u64,
    parameters: Option<Operation>,
) -> Result<Hash, Error> {
    let hash = crate::hash::hash(&serialization);
    let issued_at = time.unwrap_or_else(|| OffsetDateTime::now_utc());

    ActiveModel::from(Model {
        seq,
        epoch_id: epoch.into(),
        epoch_seq,
        id: hash.into(),
        issued_at,
        serialization,
        invoker: invocation.invoker,
        resource: invocation.capability.resource,
        ability: invocation.capability.action,
        orbit: orbit.to_string(),
    })
    .save(db)
    .await?;

    if let Some(Operation::KvWrite {
        key,
        value,
        metadata,
    }) = parameters
    {
        kv::ActiveModel::from(kv::Model {
            key,
            value,
            seq,
            epoch_id: epoch.into(),
            invocation_id: hash.into(),
            orbit: orbit.to_string(),
            metadata,
        })
        .save(db)
        .await?;
    }

    Ok(hash)
}
