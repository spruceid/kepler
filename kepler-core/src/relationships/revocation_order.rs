use super::super::models::epoch;
use crate::hash::Hash;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "revocation_order")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub orbit: String,
    #[sea_orm(primary_key)]
    pub epoch: Hash,
    #[sea_orm(primary_key)]
    pub epoch_seq: String,

    pub revocation: Hash,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "epoch::Entity",
        from = "(Column::Epoch, Column::Orbit)",
        to = "(epoch::Column::Id, epoch::Column::Orbit)"
    )]
    Epoch,
    #[sea_orm(
        belongs_to = "revocation::Entity",
        from = "Column::revocation",
        to = "revocation::Column::Id"
    )]
    Revocation,
}

impl Related<epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
    }
}

impl Related<revocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Revocation.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
