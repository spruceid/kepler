use sea_orm_migration::prelude::*;
pub mod m20230510_101010_init_tables;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20230510_101010_init_tables::Migration)]
    }
}
