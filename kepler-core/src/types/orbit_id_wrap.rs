use kepler_lib::resource::OrbitId;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Eq, Hash, PartialOrd, Ord)]
pub struct OrbitIdWrap(pub OrbitId);

impl From<OrbitId> for OrbitIdWrap {
    fn from(id: OrbitId) -> Self {
        Self(id)
    }
}

impl From<OrbitIdWrap> for OrbitId {
    fn from(id: OrbitIdWrap) -> Self {
        id.0
    }
}

impl AsRef<OrbitId> for OrbitIdWrap {
    fn as_ref(&self) -> &OrbitId {
        &self.0
    }
}

impl core::borrow::Borrow<OrbitId> for OrbitIdWrap {
    fn borrow(&self) -> &OrbitId {
        &self.0
    }
}

impl PartialEq<OrbitId> for OrbitIdWrap {
    fn eq(&self, other: &OrbitId) -> bool {
        self.0 == *other
    }
}

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
            Value::String(Some(x)) => Ok(OrbitId::from_str(&x)
                .map_err(|_| sea_orm::sea_query::ValueTypeErr)?
                .into()),
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
