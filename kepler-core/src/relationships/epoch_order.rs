use crate::hash::Hash;
use crate::models::*;
use crate::types::OrbitIdWrap;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "epoch_order")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub parent: Hash,
    #[sea_orm(primary_key)]
    pub child: Hash,
    #[sea_orm(primary_key)]
    pub orbit: OrbitIdWrap,
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
    #[sea_orm(
        belongs_to = "orbit::Entity",
        from = "Column::Orbit",
        to = "orbit::Column::Id"
    )]
    Orbit,
}

impl Related<epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Parent.def()
    }
}

impl Related<orbit::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Orbit.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
