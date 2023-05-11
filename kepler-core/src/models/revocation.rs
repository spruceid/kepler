use super::super::{events::Revocation, models::*, util};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "revocation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Vec<u8>,
    pub serialized: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "actor::Entity",
        from = "Column::Id",
        to = "actor::Column::Id"
    )]
    Revoker,
    #[sea_orm(
        belongs_to = "epoch::Entity",
        from = "Column::Id",
        to = "epoch::Column::Id"
    )]
    Epoch,
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "Column::Id",
        to = "delegation::Column::Id"
    )]
    Delegation,
}

impl Related<actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Revoker.def()
    }
}

impl Related<epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
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
    #[error(transparent)]
    ParameterExtraction(#[from] util::RevocationError),
    #[error("Invocation expired or not yet valid")]
    InvalidTime,
    #[error("Failed to verify signature")]
    InvalidSignature,
    #[error("Unauthorized Revoker")]
    UnauthorizedRevoker(String),
    #[error("Cannot find parent delegation")]
    MissingParents,
}

pub async fn process<C: ConnectionTrait>(
    root: &str,
    db: &C,
    revocation: Revocation,
) -> Result<[u8; 32], Error> {
    todo!()
}
