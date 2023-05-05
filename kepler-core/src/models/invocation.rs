use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "invocation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Vec<u8>,
    pub issued_at: OffsetDateTime,
    pub serialized: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // inverse relation, invocations belong to parent delegations
    #[sea_orm(
        belongs_to = "super::delegation::Entity",
        from = "Column::Id",
        to = "super::delegation::Column::Id"
    )]
    Parent,
    // inverse relation, invocations belong to invokers
    #[sea_orm(
        belongs_to = "super::actor::Entity",
        from = "Column::Id",
        to = "super::actor::Column::Id"
    )]
    Invoker,
    // inverse relation, invocations belong to epochs
    #[sea_orm(
        belongs_to = "super::epoch::Entity",
        from = "Column::Id",
        to = "super::epoch::Column::Id"
    )]
    Epoch,
}

impl Related<super::delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Parent.def()
    }
}

impl Related<super::actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invoker.def()
    }
}

impl Related<super::epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
