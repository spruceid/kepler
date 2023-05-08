use crate::events::{Delegation, Event, Invocation, Revocation};
use crate::models::*;
use sea_orm::{
    entity::prelude::*, error::DbErr, query::QuerySelect, ConnectOptions, ConnectionTrait,
    Database, DatabaseConnection, DatabaseTransaction, Schema, TransactionTrait,
};

pub struct OrbitDatabase {
    conn: DatabaseConnection,
    orbit: OrbitId,
    root: String,
}

pub struct Commit {
    pub hash: [u8; 32],
    pub seq: u64,
    pub commited_events: Vec<[u8; 32]>,
}

#[derive(Debug, thiserror::Error)]
pub enum TxError {
    #[error("database error: {0}")]
    Db(#[from] DbErr),
    #[error(transparent)]
    Ucan(#[from] ssi::ucan::Error),
    #[error(transparent)]
    Cacao(#[from] kepler_lib::cacaos::siwe_cacao::VerificationError),
    #[error("invalid delegation")]
    InvalidDelegation,
}

impl OrbitDatabase {
    pub async fn new<C: Into<ConnectOptions>>(options: C) -> Result<Self, DbErr> {
        Ok(Self {
            conn: Database::connect(options).await?,
        })
    }

    pub async fn setup_tables(&self) -> Result<(), DbErr> {
        let db_backend = self.conn.get_database_backend();
        let schema = Schema::new(db_backend);
        self.conn
            .execute(db_backend.build(&schema.create_table_from_entity(epoch::Entity)))
            .await?;
        self.conn
            .execute(db_backend.build(&schema.create_table_from_entity(delegation::Entity)))
            .await?;
        self.conn
            .execute(db_backend.build(&schema.create_table_from_entity(invocation::Entity)))
            .await?;
        self.conn
            .execute(db_backend.build(&schema.create_table_from_entity(revocation::Entity)))
            .await?;
        self.conn
            .execute(db_backend.build(&schema.create_table_from_entity(actor::Entity)))
            .await?;
        Ok(())
    }

    pub async fn get_max_seq(&self) -> Result<u64, DbErr> {
        Ok(epoch::Entity::find()
            .select_only()
            .column_as(epoch::Column::Seq.max(), "seq")
            .into_tuple()
            .one(&self.conn)
            .await?
            .unwrap_or(0))
    }

    pub async fn get_most_recent(&self) -> Result<Vec<[u8; 32]>, DbErr> {
        Ok(todo!("get unconsumed latest tx from db"))
    }

    pub async fn transact(&self, events: Vec<Event>) -> Result<Commit, TxError> {
        let tx = self
            .conn
            .begin_with_config(Some(sea_orm::IsolationLevel::ReadUncommitted), None)
            .await?;
        let mut commited_events = Vec::new();
        for event in events {
            commited_events.push(match event {
                // dropping tx rolls back changes, so fine to '?' here
                Event::Delegation(d) => self.delegate_tx(&tx, d).await?,
                Event::Invocation(i) => self.invoke_tx(&tx, i).await?,
                Event::Revocation(r) => self.revoke_tx(&tx, r).await?,
            });
        }
        // TODO update epoch table
        let seq = self.get_max_seq().await? + 1;
        let parents = self.get_most_recent().await?;

        tx.commit().await?;
        todo!()
    }

    async fn delegate_tx(
        &self,
        tx: &DatabaseTransaction,
        delegation: Delegation,
    ) -> Result<[u8; 32], TxError> {
        delegation::process(tx, delegation).await
    }

    async fn invoke_tx(
        &self,
        tx: &DatabaseTransaction,
        invocation: Invocation,
    ) -> Result<[u8; 32], TxError> {
        todo!()
    }

    async fn revoke_tx(
        &self,
        tx: &DatabaseTransaction,
        revocation: Revocation,
    ) -> Result<[u8; 32], TxError> {
        todo!()
    }

    // to allow users to make custom read queries
    async fn readable(&self) -> Result<DatabaseTransaction, DbErr> {
        self.conn
            .begin_with_config(None, Some(sea_orm::AccessMode::ReadOnly))
            .await
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use async_std::test;

    async fn get_db() -> Result<OrbitDatabase, DbErr> {
        OrbitDatabase::new("sqlite::memory:").await
    }

    #[test]
    async fn basic() {
        let db = get_db().await.unwrap();
    }
}
