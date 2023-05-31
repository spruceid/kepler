use super::*;
use crate::hash::Hash;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "kv_delete")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub invocation_id: Hash,
    #[sea_orm(primary_key)]
    pub orbit: String,
    #[sea_orm(primary_key)]
    pub seq: i64,
    #[sea_orm(primary_key)]
    pub epoch_id: Hash,

    pub key: String,
    pub deleted_seq: i64,
    pub deleted_epoch_id: Hash,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "invocation::Entity",
        from = "(Column::InvocationId, Column::Orbit)",
        to = "(invocation::Column::Id, invocation::Column::Orbit)"
    )]
    Invocation,
    #[sea_orm(
        belongs_to = "kv_write::Entity",
        from = "(Column::Orbit, Column::DeletedEpochId, Column::DeletedSeq)",
        to = "(kv_write::Column::Orbit, kv_write::Column::EpochId, kv_write::Column::Seq)"
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
