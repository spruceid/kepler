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
            .create_table(schema.create_table_from_entity(orbit::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(actor::Entity))
            .await?;
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
            .create_table(schema.create_table_from_entity(event_order::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(parent_delegations::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(abilities::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(kv_write::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(kv_delete::Entity))
            .await?;

        manager
            .create_table(schema.create_table_from_entity(epoch_order::Entity))
            .await?;
        manager
            .create_table(schema.create_table_from_entity(invoked_abilities::Entity))
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(orbit::Entity).to_owned())
            .await?;
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
            .drop_table(Table::drop().table(kv_write::Entity).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(event_order::Entity).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(epoch_order::Entity).to_owned())
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
