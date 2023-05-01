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
    #[sea_orm(
        belongs_to = "super::delegation::Entity",
        from = "Column::Id",
        to = "super::delegation::Column::Id"
    )]
    DelegatedBy,
    #[sea_orm(has_many = "super::invocation::Entity")]
    InvokerOf,
    #[sea_orm(has_many = "super::revocation::Entity")]
    RevokerOf,
}

impl Related<super::invocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::InvokerOf.def()
    }
}

impl Related<super::revocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::RevokerOf.def()
    }
}

impl Related<super::delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::DelegatorOf.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
