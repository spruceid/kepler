use sea_orm::entity::prelude::*;
use time::OffsetDateTime;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "delegation")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: u64,
    pub expiry: OffsetDateTime,
    pub issued_at: OffsetDateTime,
    pub not_before: OffsetDateTime,
    pub nonce: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_one = "super::actor::Entity")]
    Delegator,
    #[sea_orm(has_one = "super::actor::Entity")]
    Delegatee,
    #[sea_orm(has_one = "super::epoch::Entity")]
    Epoch,
    #[sea_orm(has_many = "super::invocation::Entity")]
    Invocation,
    #[sea_orm(has_many = "super::revocation::Entity")]
    Recovation,
}

impl Related<super::epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
