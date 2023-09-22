use kepler_lib::ssi::ucan::capabilities::Ability;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct AbilityName(pub Ability);

impl AsRef<Ability> for AbilityName {
    fn as_ref(&self) -> &Ability {
        &self.0
    }
}

impl From<Ability> for AbilityName {
    fn from(id: Ability) -> Self {
        AbilityName(id)
    }
}

impl From<AbilityName> for Ability {
    fn from(id: AbilityName) -> Self {
        id.0
    }
}

impl From<AbilityName> for Value {
    fn from(r: AbilityName) -> Self {
        Value::String(Some(Box::new(r.to_string())))
    }
}

impl PartialEq<Ability> for AbilityName {
    fn eq(&self, other: &Ability) -> bool {
        self.0 == *other
    }
}

impl AbilityName {
    pub fn into_inner(self) -> Ability {
        self.0
    }
}

impl sea_orm::TryGetable for AbilityName {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        Ok(res
            .try_get_by::<String, I>(idx)?
            .parse()
            .map_err(|e| DbErr::TryIntoErr {
                from: "String",
                into: "AbilityName",
                source: Box::new(e),
            })?)
    }
}

impl sea_orm::sea_query::ValueType for AbilityName {
    fn try_from(v: Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            Value::String(Some(x)) => x.parse().or(Err(sea_orm::sea_query::ValueTypeErr)),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        stringify!(AbilityName).to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::String
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::String(None)
    }
}

impl std::str::FromStr for AbilityName {
    type Err = <Ability as std::str::FromStr>::Err;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Ability::from_str(s)?))
    }
}

impl std::fmt::Display for AbilityName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl sea_orm::sea_query::Nullable for AbilityName {
    fn null() -> Value {
        Value::String(None)
    }
}

impl sea_orm::TryFromU64 for AbilityName {
    fn try_from_u64(_: u64) -> Result<Self, DbErr> {
        Err(DbErr::ConvertFromU64(stringify!($type)))
    }
}
