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
    // inverse relation, delegations belong to delegators
    #[sea_orm(
        belongs_to = "super::actor::Entity",
        from = "Column::Id",
        to = "super::actor::Column::Id"
    )]
    Delegator,
    #[sea_orm(has_one = "super::actor::Entity")]
    Delegatee,
    // inverse relation, delegations belong to epochs
    #[sea_orm(
        belongs_to = "super::epoch::Entity",
        from = "Column::Id",
        to = "super::epoch::Column::Id"
    )]
    Epoch,
    // inverse relation, delegations belong to their parent delegations
    #[sea_orm(belongs_to = "Entity", from = "Column::Id", to = "Column::Id")]
    Parent,
    #[sea_orm(has_many = "super::invocation::Entity")]
    Invocation,
    #[sea_orm(has_many = "super::revocation::Entity")]
    Revocation,
}

impl Related<super::actor::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegator.def()
    }
}

impl Related<super::epoch::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Epoch.def()
    }
}

impl Related<Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Parent.def()
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
