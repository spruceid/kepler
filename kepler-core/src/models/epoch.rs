use super::*;
use crate::hash::Hash;
use crate::relationships::*;
use kepler_lib::resource::OrbitId;
use sea_orm::entity::prelude::*;
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, PartialOrd, Ord)]
#[sea_orm(table_name = "epoch")]
pub struct Model {
    /// Sequence number
    pub seq: i64,
    /// Hash-based ID
    #[sea_orm(primary_key)]
    pub id: Hash,

    #[sea_orm(primary_key)]
    pub orbit: epoch::OrbitIdWrap,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrbitIdWrap(pub OrbitId);

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "event_order::Entity")]
    Events,
    #[sea_orm(has_many = "epoch_order::Entity")]
    Children,
}

impl Related<epoch_order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Children.def()
    }
}

impl Related<event_order::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Events.def()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ParentToChild;

impl Linked for ParentToChild {
    type FromEntity = Entity;

    type ToEntity = Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            epoch_order::Relation::Parent.def().rev(),
            epoch_order::Relation::Child.def(),
        ]
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ChildToParent;

impl Linked for ChildToParent {
    type FromEntity = Entity;

    type ToEntity = Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            epoch_order::Relation::Child.def().rev(),
            epoch_order::Relation::Parent.def(),
        ]
    }
}

impl ActiveModelBehavior for ActiveModel {}

impl From<OrbitIdWrap> for Value {
    fn from(o: OrbitIdWrap) -> Self {
        Value::String(Some(Box::new(o.0.to_string())))
    }
}

impl sea_orm::TryGetable for OrbitIdWrap {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        let s: String = res.try_get_by(idx).map_err(sea_orm::TryGetError::DbErr)?;
        Ok(OrbitIdWrap(OrbitId::from_str(&s).map_err(|e| {
            sea_orm::TryGetError::DbErr(DbErr::TryIntoErr {
                from: "String",
                into: "OrbitId",
                source: Box::new(e),
            })
        })?))
    }
}

impl sea_orm::sea_query::ValueType for OrbitIdWrap {
    fn try_from(v: Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            Value::String(Some(x)) => Ok(<OrbitIdWrap as std::str::FromStr>::from_str(&x)
                .map_err(|_| sea_orm::sea_query::ValueTypeErr)?),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        stringify!(OrbitId).to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::String
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::String(None)
    }
}

impl sea_orm::sea_query::Nullable for OrbitIdWrap {
    fn null() -> Value {
        Value::String(None)
    }
}

impl sea_orm::TryFromU64 for OrbitIdWrap {
    fn try_from_u64(_: u64) -> Result<Self, DbErr> {
        Err(DbErr::ConvertFromU64(stringify!($type)))
    }
}
