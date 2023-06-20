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

#[derive(Copy, Clone, Debug)]
pub struct Ordering;

impl Linked for Ordering {
    type FromEntity = Entity;

    type ToEntity = event_order::Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            Relation::Invocation.def(),
            invocation::Relation::Ordering.def(),
        ]
    }
}

impl ActiveModelBehavior for ActiveModel {}
