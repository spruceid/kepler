use super::super::models::*;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "actor")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, unique)]
    pub id: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "delegation::Entity")]
    DelegatorOf,
    #[sea_orm(has_many = "delegation::Entity")]
    DelegatedBy,
    #[sea_orm(has_many = "invocation::Entity")]
    InvokerOf,
    #[sea_orm(has_many = "revocation::Entity")]
    RevokerOf,
}

impl Related<invocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::InvokerOf.def()
    }
}

impl Related<revocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::RevokerOf.def()
    }
}

impl Related<delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::DelegatorOf.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
