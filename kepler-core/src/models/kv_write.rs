use crate::hash::Hash;
use crate::types::{Metadata, OrbitIdWrap};
use crate::{models::*, relationships::*};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "kv_write")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub orbit: OrbitIdWrap,
    #[sea_orm(primary_key)]
    pub key: String,
    #[sea_orm(primary_key)]
    pub invocation: Hash,
    pub seq: i64,
    pub epoch: Hash,
    pub epoch_seq: i64,
    pub value: Hash,
    pub metadata: Metadata,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "invocation::Entity",
        from = "Column::Invocation",
        to = "invocation::Column::Id"
    )]
    Invocation,
    #[sea_orm(has_many = "kv_delete::Entity")]
    Deleted,
    #[sea_orm(
        belongs_to = "event_order::Entity",
        from = "(Column::Epoch, Column::EpochSeq, Column::Orbit)",
        to = "(event_order::Column::Epoch, event_order::Column::EpochSeq, event_order::Column::Orbit)"
    )]
    Ordering,
}

impl Related<invocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invocation.def()
    }
}

impl Related<kv_delete::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Deleted.def()
    }
}

impl Related<event_order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Ordering.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
