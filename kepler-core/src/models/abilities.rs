use super::*;
use crate::hash::Hash;
use crate::types::{caveats::Caveats, resource::Resource};
use kepler_lib::resource::{OrbitId, ResourceId};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Display;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "ability")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub resource: Resource,
    #[sea_orm(primary_key)]
    pub ability: String,
    #[sea_orm(primary_key)]
    pub delegation: Hash,

    pub caveats: Caveats,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "Column::Delegation",
        to = "delegation::Column::Id"
    )]
    Delegation,
}

impl Related<delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegation.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
