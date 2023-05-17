use super::*;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "ability")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub resource: String,
    #[sea_orm(primary_key)]
    pub ability: String,
    #[sea_orm(primary_key)]
    pub delegation: Vec<u8>,
    #[sea_orm(primary_key)]
    pub orbit: String,

    pub caveats: Option<Caveats>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Eq, Default)]
pub struct Caveats(pub BTreeMap<String, serde_json::Value>);

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "delegation::Entity",
        from = "(Column::Delegation, Column::Orbit)",
        to = "(delegation::Column::Id, delegation::Column::Orbit)"
    )]
    Delegation,
}

impl Related<delegation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Delegation.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

impl From<Caveats> for Value {
    fn from(source: Caveats) -> Self {
        Value::Json(serde_json::to_value(&source).ok().map(Box::new))
    }
}

impl sea_orm::TryGetable for Caveats {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        let json: serde_json::Value = res.try_get_by(idx).map_err(sea_orm::TryGetError::DbErr)?;
        serde_json::from_value(json)
            .map_err(|e| sea_orm::TryGetError::DbErr(DbErr::Json(e.to_string())))
    }
}

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
