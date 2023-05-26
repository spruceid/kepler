use super::super::models::*;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "invoked_abilities")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub invocation: Vec<u8>,
    #[sea_orm(primary_key)]
    pub resource: String,
    #[sea_orm(primary_key)]
    pub ability: String,
    #[sea_orm(primary_key)]
    pub delegation: Vec<u8>,
    #[sea_orm(primary_key)]
    pub orbit: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // inverse relation, delegations belong to delegators
    // #[sea_orm(
    //     belongs_to = "invocation::Entity",
    //     from = "(Column::Invocation, Column::Orbit)",
    //     to = "(invocation::Column::Id, invocation::Column::Orbit)"
    // )]
    // Invocation,
    #[sea_orm(
        belongs_to = "abilities::Entity",
        from = "(Column::Resource, Column::Ability, Column::Delegation)",
        to = "(abilities::Column::Resource, abilities::Column::Ability, abilities::Column::Delegation)"
    )]
    Ability,
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "(Column::Delegation, Column::Orbit)",
        to = "(delegation::Column::Id, delegation::Column::Orbit)"
    )]
    Delegation,
}

impl ActiveModelBehavior for ActiveModel {}
