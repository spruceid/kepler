use super::super::models::*;
use crate::hash::Hash;
use crate::types::Resource;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "invoked_abilities")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub invocation: Hash,
    #[sea_orm(primary_key)]
    pub resource: Resource,
    #[sea_orm(primary_key)]
    pub ability: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // inverse relation, delegations belong to delegators
    #[sea_orm(
        belongs_to = "invocation::Entity",
        from = "Column::Invocation",
        to = "invocation::Column::Id"
    )]
    Invocation,
}

impl Related<invocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invocation.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
