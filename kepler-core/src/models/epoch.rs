use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "epoch")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Vec<u8>,
    pub seq: u64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(belongs_to = "Entity", from = "Column::Id", to = "Column::Id")]
    Parent,
    // #[sea_orm(has_many = "Entity")]
    // Child,
    #[sea_orm(has_many = "super::delegation::Entity")]
    Delegation,
    #[sea_orm(has_many = "super::invocation::Entity")]
    Invocation,
    #[sea_orm(has_many = "super::revocation::Entity")]
    Revocation,
}

// impl Related<Entity> for Entity {
//     fn to() -> RelationDef {
//         Relation::Child.def()
//     }
// }

impl Related<super::delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegation.def()
    }
}

impl Related<super::invocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invocation.def()
    }
}

impl Related<super::revocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Revocation.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
