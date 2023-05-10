use super::super::models::*;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "invoked_abilities")]
pub struct Model {
    #[sea_orm(primary_key)]
    invocation: Vec<u8>,
    #[sea_orm(primary_key)]
    resource: String,
    #[sea_orm(primary_key)]
    action_namespace: String,
    #[sea_orm(primary_key)]
    action: String,
    #[sea_orm(primary_key)]
    delegation: Vec<u8>,
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
    #[sea_orm(
        belongs_to = "abilities::Entity",
        from = "(Column::Resource, Column::ActionNamespace, Column::Action, Column::Delegation)",
        to = "(abilities::Column::Resource, abilities::Column::ActionNamespace, abilities::Column::Action, abilities::Column::Delegation)"
    )]
    Ability,
}

impl ActiveModelBehavior for ActiveModel {}
