use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "epochs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub seq: u64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub hash: [u8; 32],
}
