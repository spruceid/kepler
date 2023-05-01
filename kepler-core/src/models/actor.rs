use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "actor")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::delegation::Entity")]
    DelegatorOf,
    #[sea_orm(has_many = "super::delegation::Entity")]
    DelegateeOf,
    #[sea_orm(has_many = "super::invocation::Entity")]
    InvokerOf,
}

impl ActiveModelBehavior for ActiveModel {}
