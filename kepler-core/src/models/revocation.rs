use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "revocation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Vec<u8>,
    pub serialized: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::actor::Entity",
        from = "Column::Id",
        to = "super::actor::Column::Id"
    )]
    Revoker,
    #[sea_orm(
        belongs_to = "super::epoch::Entity",
        from = "Column::Id",
        to = "super::epoch::Column::Id"
    )]
    Epoch,
    #[sea_orm(
        belongs_to = "super::delegation::Entity",
        from = "Column::Id",
        to = "super::delegation::Column::Id"
    )]
    Delegation,
}

impl Related<super::actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Revoker.def()
    }
}

impl Related<super::epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
    }
}

impl Related<super::delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegation.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
