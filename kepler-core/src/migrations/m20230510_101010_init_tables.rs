use crate::{models::*, relationships::*};
use sea_orm::Schema;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let schema = Schema::new(manager.get_database_backend());

        manager
            .create_table(schema.create_table_from_entity(epoch::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(delegation::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(invocation::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(revocation::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(actor::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(abilities::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(kv::Entity))
            .await?;

        manager
            .create_table(schema.create_table_from_entity(epochs::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(invoked_abilities::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(parent_delegations::Entity))
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(epoch::Entity).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(delegation::Entity).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(invocation::Entity).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(revocation::Entity).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(actor::Entity).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(abilities::Entity).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(kv::Entity).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(epochs::Entity).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(invoked_abilities::Entity).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(parent_delegations::Entity).to_owned())
            .await?;

        Ok(())
    }
}
