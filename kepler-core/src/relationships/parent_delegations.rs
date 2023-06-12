use super::super::models::delegation;
use crate::hash::Hash;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "parent_delegation")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub parent: Hash,
    #[sea_orm(primary_key)]
    pub child: Hash,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "Column::Parent",
        to = "delegation::Column::Id"
    )]
    Parent,
    #[sea_orm(has_many = "delegation::Entity")]
    Child,
}

impl Related<delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Parent.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
