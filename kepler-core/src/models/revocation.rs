use super::super::{events::Revocation, models::*, relationships::*};
use crate::hash::{hash, Hash};
use kepler_lib::authorization::KeplerRevocation;
use sea_orm::{entity::prelude::*, ConnectionTrait};
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "revocation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, unique)]
    pub id: Hash,

    pub revoker: String,
    pub revoked: Hash,
    pub serialization: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "actor::Entity",
        from = "Column::Revoker",
        to = "actor::Column::Id"
    )]
    Revoker,
    #[sea_orm(
        belongs_to = "event_order::Entity",
        from = "Column::Id",
        to = "event_order::Column::Event"
    )]
    Ordering,
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "Column::Revoked",
        to = "delegation::Column::Id"
    )]
    Delegation,
    #[sea_orm(
        belongs_to = "parent_delegations::Entity",
        from = "Column::Id",
        to = "parent_delegations::Column::Child"
    )]
    Parents,
}

impl Related<actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Revoker.def()
    }
}

impl Related<event_order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Ordering.def()
    }
}

impl Related<delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegation.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Db(#[from] DbErr),
    #[error(transparent)]
    InvalidRevocation(#[from] RevocationError),
}

#[derive(Debug, thiserror::Error)]
pub enum RevocationError {
    #[error("Revocation expired or not yet valid")]
    InvalidTime,
    #[error("Failed to verify signature")]
    InvalidSignature,
    #[error("Unauthorized Revoker")]
    UnauthorizedRevoker(String),
    #[error("Cannot find parent delegation")]
    MissingParents,
}

pub(crate) async fn process<C: ConnectionTrait>(
    db: &C,
    revocation: Revocation,
) -> Result<Hash, Error> {
    let (r, serialization) = (revocation.0, revocation.1);

    let t = OffsetDateTime::now_utc();

    match &r.revocation {
        KeplerRevocation::Cacao(c) => {
            c.verify()
                .await
                .map_err(|_| RevocationError::InvalidSignature)?;
            if !c.payload().valid_at(&t) {
                return Err(RevocationError::InvalidTime.into());
            };
        }
    };

    let hash: Hash = hash(&serialization);
    let delegation = delegation::Entity::find_by_id(Hash::from(r.revoked))
        .one(db)
        .await?
        .ok_or(RevocationError::MissingParents)?;

    // check the revoker is also the delegator
    if delegation.delegator != r.revoker {
        return Err(RevocationError::UnauthorizedRevoker(r.revoker).into());
    };

    Entity::insert(ActiveModel::from(Model {
        id: hash,
        serialization,
        revoker: r.revoker,
        revoked: delegation.id,
    }))
    .exec(db)
    .await?;

    for parent in r.parents {
        parent_delegations::Entity::insert(parent_delegations::ActiveModel::from(
            parent_delegations::Model {
                child: hash,
                parent: parent.into(),
            },
        ))
        .exec(db)
        .await?;
    }

    Ok(hash)
}
