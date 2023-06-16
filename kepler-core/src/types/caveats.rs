use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Eq, Default)]
pub struct Caveats(pub BTreeMap<String, serde_json::Value>);

impl From<Caveats> for Value {
    fn from(source: Caveats) -> Self {
        Value::Json(serde_json::to_value(source).ok().map(Box::new))
    }
}

impl sea_orm::TryGetableFromJson for Caveats {}

impl sea_orm::sea_query::ValueType for Caveats {
    fn try_from(v: Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            Value::Json(Some(x)) => Ok(Caveats(
                serde_json::from_value(*x).map_err(|_| sea_orm::sea_query::ValueTypeErr)?,
            )),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        stringify!(Caveats).to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::Json
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Json
    }
}

impl sea_orm::sea_query::Nullable for Caveats {
    fn null() -> Value {
        Value::Json(None)
    }
}
