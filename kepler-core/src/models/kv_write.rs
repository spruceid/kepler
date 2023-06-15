use crate::hash::Hash;
use crate::types::orbit_id_wrap::OrbitIdWrap;
use crate::{models::*, relationships::*};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "kv_write")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub orbit: OrbitIdWrap,
    #[sea_orm(primary_key)]
    pub key: String,
    #[sea_orm(primary_key)]
    pub invocation: Hash,
    pub value: Hash,
    pub metadata: Metadata,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Eq, PartialOrd, Ord, Hash)]
pub struct Metadata(pub BTreeMap<String, String>);

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "invocation::Entity",
        from = "Column::Invocation",
        to = "invocation::Column::Id"
    )]
    Invocation,
    #[sea_orm(has_many = "kv_delete::Entity")]
    Deleted,
}

impl Related<invocation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Invocation.def()
    }
}

impl Related<kv_delete::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Deleted.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

impl From<Metadata> for Value {
    fn from(source: Metadata) -> Self {
        Value::Json(serde_json::to_value(source).ok().map(Box::new))
    }
}

impl sea_orm::TryGetable for Metadata {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        let json: serde_json::Value = res.try_get_by(idx).map_err(sea_orm::TryGetError::DbErr)?;
        serde_json::from_value(json)
            .map_err(|e| sea_orm::TryGetError::DbErr(DbErr::Json(e.to_string())))
    }
}

impl sea_orm::sea_query::ValueType for Metadata {
    fn try_from(v: Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            Value::Json(Some(x)) => Ok(Metadata(
                serde_json::from_value(*x).map_err(|_| sea_orm::sea_query::ValueTypeErr)?,
            )),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        stringify!(Metadata).to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::Json
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Json
    }
}

impl sea_orm::sea_query::Nullable for Metadata {
    fn null() -> Value {
        Value::Json(None)
    }
}
