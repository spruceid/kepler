use super::super::models::epoch;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "parent_epochs")]
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
    // inverse relation, delegations belong to delegators
    #[sea_orm(
        belongs_to = "epoch::Entity",
        from = "(Column::Parent, Column::Orbit)",
        to = "(epoch::Column::Id, epoch::Column::Orbit)"
    )]
    Parent,
    #[sea_orm(
        belongs_to = "epoch::Entity",
        from = "(Column::Child, Column::Orbit)",
        to = "(epoch::Column::Id, epoch::Column::Orbit)"
    )]
    Child,
}

impl ActiveModelBehavior for ActiveModel {}
