use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Eq, PartialOrd, Ord, Hash)]
pub struct Metadata(pub BTreeMap<String, String>);

impl From<Metadata> for Value {
    fn from(source: Metadata) -> Self {
        Value::Json(serde_json::to_value(source).ok().map(Box::new))
    }
}

impl sea_orm::TryGetableFromJson for Metadata {}

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
