use super::*;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "kv")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub invocation_id: Vec<u8>,
    #[sea_orm(primary_key)]
    pub key: String,
    #[sea_orm(primary_key)]
    pub orbit: String,

    pub seq: u64,
    pub epoch_id: Vec<u8>,

    pub value: Vec<u8>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "invocation::Entity",
        from = "(Column::InvocationId, Column::Orbit)",
        to = "(invocation::Column::Id, invocation::Column::Orbit)"
    )]
    Invocation,
}

impl Related<invocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invocation.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
