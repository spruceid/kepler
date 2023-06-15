use super::super::models::*;
use crate::hash::Hash;
use crate::relationships::*;
use crate::types::orbit_id_wrap::OrbitIdWrap;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, PartialOrd, Ord)]
#[sea_orm(table_name = "event_order")]
pub struct Model {
    /// Sequence number
    pub seq: i64,
    #[sea_orm(primary_key)]
    pub epoch: Hash,
    #[sea_orm(primary_key)]
    pub epoch_seq: i64,
    pub event: Hash,

    pub orbit: OrbitIdWrap,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "delegation::Entity")]
    Delegation,
    #[sea_orm(has_many = "invocation::Entity")]
    Invocation,
    #[sea_orm(has_many = "revocation::Entity")]
    Revocation,
    #[sea_orm(
        belongs_to = "epoch::Entity",
        from = "(Column::Epoch, Column::Orbit)",
        to = "(epoch::Column::Id, epoch::Column::Orbit)"
    )]
    Epoch,
}

impl Related<delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegation.def()
    }
}

impl Related<invocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invocation.def()
    }
}

impl Related<revocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Revocation.def()
    }
}

impl Related<epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
