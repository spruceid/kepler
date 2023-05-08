use super::super::models::delegation;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "parent_delegation")]
pub struct Model {
    #[sea_orm(primary_key)]
    parent: Vec<u8>,
    #[sea_orm(primary_key)]
    child: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // inverse relation, delegations belong to delegators
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "Column::Parent",
        to = "delegation::Column::Id"
    )]
    Parent,
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "Column::Child",
        to = "delegation::Column::Id"
    )]
    Child,
}

impl ActiveModelBehavior for ActiveModel {}
