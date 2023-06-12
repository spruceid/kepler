use super::*;
use crate::hash::Hash;
use crate::relationships::*;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, PartialOrd, Ord)]
#[sea_orm(table_name = "epoch")]
pub struct Model {
    /// Sequence number
    pub seq: i64,
    /// Hash-based ID
    #[sea_orm(primary_key, unique, auto_increment = false)]
    pub id: Hash,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "event_order::Entity")]
    Events,
    #[sea_orm(has_many = "epoch_order::Entity")]
    Children,
}

impl Related<epoch_order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Children.def()
    }
}

impl Related<event_order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Events.def()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ParentToChild;

impl Linked for ParentToChild {
    type FromEntity = Entity;

    type ToEntity = Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            epoch_order::Relation::Parent.def().rev(),
            epoch_order::Relation::Child.def(),
        ]
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ChildToParent;

impl Linked for ChildToParent {
    type FromEntity = Entity;

    type ToEntity = Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            epoch_order::Relation::Child.def().rev(),
            epoch_order::Relation::Parent.def(),
        ]
    }
}

impl ActiveModelBehavior for ActiveModel {}
