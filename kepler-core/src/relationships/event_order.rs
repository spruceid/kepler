use super::super::models::*;
use crate::hash::Hash;
use crate::types::OrbitIdWrap;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, PartialOrd, Ord)]
#[sea_orm(table_name = "event_order")]
pub struct Model {
    /// Sequence number
    pub seq: i64,
    #[sea_orm(primary_key)]
    pub epoch: Hash,
    /// Sequence number of the event within the epoch
    #[sea_orm(primary_key)]
    pub epoch_seq: i64,
    pub event: Hash,

    #[sea_orm(primary_key)]
    pub orbit: OrbitIdWrap,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "epoch::Entity",
        from = "(Column::Epoch, Column::Orbit)",
        to = "(epoch::Column::Id, epoch::Column::Orbit)"
    )]
    Epoch,
    #[sea_orm(has_many = "kv_write::Entity")]
    KvWrite,
    #[sea_orm(
        belongs_to = "orbit::Entity",
        from = "Column::Orbit",
        to = "orbit::Column::Id"
    )]
    Orbit,
}

impl Related<epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
    }
}

impl Related<kv_write::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::KvWrite.def()
    }
}

impl Related<orbit::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Orbit.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
