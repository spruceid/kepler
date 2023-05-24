use super::*;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "epoch")]
pub struct Model {
    /// Hash-based ID
    #[sea_orm(primary_key, unique)]
    pub id: Vec<u8>,
    /// Orbit
    #[sea_orm(primary_key)]
    pub orbit: String,

    /// Sequence number
    pub seq: u32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "delegation::Entity")]
    Delegation,
    #[sea_orm(has_many = "invocation::Entity")]
    Invocation,
    #[sea_orm(has_many = "revocation::Entity")]
    Revocation,
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

#[derive(Copy, Clone, Debug)]
pub struct ParentToChild;

impl Linked for ParentToChild {
    type FromEntity = Entity;

    type ToEntity = Entity;

    fn link(&self) -> Vec<RelationDef> {
        use super::super::relationships::epochs;
        vec![
            epochs::Relation::Parent.def().rev(),
            epochs::Relation::Child.def(),
        ]
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ChildToParent;

impl Linked for ChildToParent {
    type FromEntity = Entity;

    type ToEntity = Entity;

    fn link(&self) -> Vec<RelationDef> {
        use super::super::relationships::epochs;
        vec![
            epochs::Relation::Child.def().rev(),
            epochs::Relation::Parent.def(),
        ]
    }
}

impl ActiveModelBehavior for ActiveModel {}
