use super::*;
use crate::hash::Hash;
use crate::types::OrbitIdWrap;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "kv_delete")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub invocation_id: Hash,
    #[sea_orm(primary_key)]
    pub orbit: OrbitIdWrap,

    pub key: String,
    pub deleted_invocation_id: Hash,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "invocation::Entity",
        from = "Column::InvocationId",
        to = "invocation::Column::Id"
    )]
    Invocation,
    #[sea_orm(
        belongs_to = "kv_write::Entity",
        from = "(Column::Orbit, Column::DeletedInvocationId, Column::Key)",
        to = "(kv_write::Column::Orbit, kv_write::Column::Invocation, kv_write::Column::Key)"
    )]
    Write,
}

impl Related<invocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invocation.def()
    }
}

impl Related<kv_write::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Write.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
