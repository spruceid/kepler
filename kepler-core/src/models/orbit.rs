use crate::models::*;
use crate::relationships::*;
use crate::types::OrbitIdWrap;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, PartialOrd, Ord)]
#[sea_orm(table_name = "orbit")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, unique)]
    pub id: OrbitIdWrap,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "event_order::Entity")]
    Events,
    #[sea_orm(has_many = "epoch::Entity")]
    Epochs,
    #[sea_orm(has_many = "epoch_order::Entity")]
    EpochOrdering,
}

impl Related<epoch_order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::EpochOrdering.def()
    }
}

impl Related<event_order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Events.def()
    }
}

impl Related<epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epochs.def()
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
