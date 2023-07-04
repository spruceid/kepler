use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Eq)]
pub struct Facts(pub BTreeMap<String, JsonValue>);

impl From<Facts> for Value {
    fn from(source: Facts) -> Self {
        Value::Json(serde_json::to_value(source).ok().map(Box::new))
    }
}

impl sea_orm::TryGetableFromJson for Facts {}

impl sea_orm::sea_query::ValueType for Facts {
    fn try_from(v: Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            Value::Json(Some(x)) => Ok(Facts(
                serde_json::from_value(*x).map_err(|_| sea_orm::sea_query::ValueTypeErr)?,
            )),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        stringify!(Facts).to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::Json
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Json
    }
}

impl sea_orm::sea_query::Nullable for Facts {
    fn null() -> Value {
        Value::Json(None)
    }
}
