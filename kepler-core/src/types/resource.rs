use kepler_lib::resource::{OrbitId, ResourceId};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use std::{fmt::Display, str::FromStr};

#[derive(Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[serde(untagged)]
pub enum Resource {
    Kepler(ResourceId),
    Other(String),
}

impl Resource {
    pub fn orbit(&self) -> Option<&OrbitId> {
        match self {
            Resource::Kepler(id) => Some(id.orbit()),
            Resource::Other(_) => None,
        }
    }

    pub fn extends(&self, other: &Self) -> bool {
        match (self, other) {
            (Resource::Kepler(a), Resource::Kepler(b)) => a.extends(b).is_ok(),
            (Resource::Other(a), Resource::Other(b)) => a.starts_with(b),
            _ => false,
        }
    }

    pub fn kepler_resource(&self) -> Option<&ResourceId> {
        match self {
            Resource::Kepler(id) => Some(id),
            Resource::Other(_) => None,
        }
    }
}

impl From<ResourceId> for Resource {
    fn from(id: ResourceId) -> Self {
        Resource::Kepler(id)
    }
}

impl From<Resource> for Value {
    fn from(r: Resource) -> Self {
        Value::String(Some(Box::new(match r {
            Resource::Kepler(k) => k.to_string(),
            Resource::Other(o) => o,
        })))
    }
}

impl sea_orm::TryGetable for Resource {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        let s: String = res.try_get_by(idx).map_err(sea_orm::TryGetError::DbErr)?;
        Ok(Resource::from(s))
    }
}

impl sea_orm::sea_query::ValueType for Resource {
    fn try_from(v: Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            Value::String(Some(x)) => Ok(Resource::from(*x)),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        stringify!(Resource).to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::String
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::String(None)
    }
}

impl From<String> for Resource {
    fn from(s: String) -> Self {
        if let Ok(resource_id) = ResourceId::from_str(&s) {
            Resource::Kepler(resource_id)
        } else {
            Resource::Other(s)
        }
    }
}

impl Display for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Resource::Kepler(resource_id) => write!(f, "{}", resource_id),
            Resource::Other(s) => write!(f, "{}", s),
        }
    }
}

impl sea_orm::sea_query::Nullable for Resource {
    fn null() -> Value {
        Value::String(None)
    }
}

impl sea_orm::TryFromU64 for Resource {
    fn try_from_u64(_: u64) -> Result<Self, DbErr> {
        Err(DbErr::ConvertFromU64(stringify!($type)))
    }
}
