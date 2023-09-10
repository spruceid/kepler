use kepler_lib::{
    iri_string::types::{UriStr, UriString},
    resource::{AnyResource, ResourceId},
};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use std::{fmt::Display, str::FromStr};

#[derive(Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Resource(pub AnyResource);

impl AsRef<AnyResource> for Resource {
    fn as_ref(&self) -> &AnyResource {
        &self.0
    }
}

impl Resource {
    pub fn extends<O: AsRef<str>, S: AsRef<AnyResource<O>>>(&self, other: &S) -> bool {
        match (self.0, other.as_ref()) {
            (AnyResource::Kepler(a), AnyResource::Kepler(b)) => a.extends(b).is_ok(),
            (AnyResource::Other(a), AnyResource::Other(b)) => a.as_str().starts_with(b.as_ref()),
            _ => false,
        }
    }
}

impl From<ResourceId> for Resource {
    fn from(id: ResourceId) -> Self {
        Resource(id.into())
    }
}

impl From<UriString> for Resource {
    fn from(id: UriString) -> Self {
        Resource(id.into())
    }
}

impl From<&UriString> for Resource {
    fn from(id: &UriString) -> Self {
        Resource(id.into())
    }
}

impl From<&UriStr> for Resource {
    fn from(id: &UriStr) -> Self {
        Resource(id.into())
    }
}

impl From<AnyResource<UriString>> for Resource {
    fn from(id: AnyResource<UriString>) -> Self {
        Resource(id.into())
    }
}

impl<'a> From<AnyResource<&'a UriStr>> for Resource {
    fn from(id: AnyResource<&'a UriStr>) -> Self {
        Resource(id.into())
    }
}

impl From<Resource> for Value {
    fn from(r: Resource) -> Self {
        Value::String(Some(Box::new(r.to_string())))
    }
}

impl sea_orm::TryGetable for Resource {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        Ok(res
            .try_get_by::<String, I>(idx)?
            .parse()
            .map_err(|e| DbErr::TryIntoErr {
                from: "String",
                into: "Resource",
                source: Box::new(e),
            })?)
    }
}

impl sea_orm::sea_query::ValueType for Resource {
    fn try_from(v: Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            Value::String(Some(s)) => s.parse().or(Err(sea_orm::sea_query::ValueTypeErr)),
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

impl FromStr for Resource {
    type Err = <AnyResource as FromStr>::Err;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(AnyResource::from_str(s)?))
    }
}

impl Display for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_ref())
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
