use super::super::{events::Invocation, models::*, util};
use kepler_lib::resolver::DID_METHODS;
use sea_orm::{entity::prelude::*, sea_query::Condition, ConnectionTrait};
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "invocation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, unique)]
    pub id: Vec<u8>,
    pub issued_at: OffsetDateTime,
    pub serialized: Vec<u8>,
    pub resource: String,
    pub action_namespace: String,
    pub action: String,
    pub parameters: Vec<u8>,
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
        from = "Column::Id",
        to = "actor::Column::Id"
    )]
    Invoker,
    // inverse relation, invocations belong to epochs
    #[sea_orm(
        belongs_to = "epoch::Entity",
        from = "Column::Id",
        to = "epoch::Column::Id"
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
    db: &C,
    invocation: Invocation,
) -> Result<[u8; 32], Error> {
    let Invocation(i, serialized, parameters) = invocation;
    i.verify_signature(DID_METHODS.to_resolver())
        .await
        .map_err(|_| InvocationError::InvalidSignature)?;
    i.payload
        .validate_time(None)
        .map_err(|_| InvocationError::InvalidTime)?;

    let i_info = util::InvocationInfo::try_from(i).map_err(InvocationError::ParameterExtraction)?;

    let now = OffsetDateTime::now_utc();
    if !i_info.parents.is_empty() || i_info.invoker.starts_with(root) {
        let parents = delegation::Entity::find()
            .filter(i_info.parents.iter().fold(Condition::any(), |cond, p| {
                cond.add(Column::Id.eq(p.to_bytes()))
            }))
            .all(db)
            .await?;
        if parents.len() != i_info.parents.len() {
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
            if delegatee.id != i_info.invoker {
                return Err(InvocationError::UnauthorizedInvoker(i_info.invoker))?;
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
            i_info.capability.resource.starts_with(&pab.resource)
                && i_info.capability.action == pab.action
        }) {
            return Err(InvocationError::UnauthorizedCapability(
                i_info.capability.resource.clone(),
                i_info.capability.action.clone(),
            ))?;
        }
    }

    let hash: [u8; 32] = blake3::hash(&serialized).into();

    ActiveModel::from(Model {
        id: hash.clone().into(),
        issued_at: now,
        serialized,
        resource: i_info.capability.resource,
        action_namespace: "".to_string(),
        action: i_info.capability.action,
        parameters,
    })
    .save(db)
    .await?;

    Ok(hash)
}
