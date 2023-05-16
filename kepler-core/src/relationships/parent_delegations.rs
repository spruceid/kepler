use super::super::models::delegation;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "parent_delegation")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub parent: Vec<u8>,
    #[sea_orm(primary_key)]
    pub child: Vec<u8>,
    #[sea_orm(primary_key)]
    pub orbit: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "(Column::Parent, Column::Orbit)",
        to = "(delegation::Column::Id, delegation::Column::Orbit)"
    )]
    Parent,
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "(Column::Child, Column::Orbit)",
        to = "(delegation::Column::Id, delegation::Column::Orbit)"
    )]
    Child,
}

impl ActiveModelBehavior for ActiveModel {}
