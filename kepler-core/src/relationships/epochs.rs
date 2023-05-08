use super::super::models::epoch;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "parent_epochs")]
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
        belongs_to = "epoch::Entity",
        from = "Column::Parent",
        to = "epoch::Column::Id"
    )]
    Parent,
    #[sea_orm(
        belongs_to = "epoch::Entity",
        from = "Column::Child",
        to = "epoch::Column::Id"
    )]
    Child,
}

impl ActiveModelBehavior for ActiveModel {}
