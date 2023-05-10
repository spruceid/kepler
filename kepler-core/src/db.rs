use crate::events::{Delegation, Event, Invocation, Revocation};
use crate::models::*;
use kepler_lib::resource::OrbitId;
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
    #[error(transparent)]
    InvalidDelegation(#[from] delegation::DelegationError),
    #[error(transparent)]
    InvalidInvocation(#[from] invocation::InvocationError),
}

impl OrbitDatabase {
    pub async fn new<C: Into<ConnectOptions>>(options: C, orbit: OrbitId) -> Result<Self, DbErr> {
        Ok(Self {
            conn: Database::connect(options).await?,
            root: orbit.did(),
            orbit,
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
        max_seq(&self.conn).await
    }

    pub async fn get_most_recent(&self) -> Result<Vec<[u8; 32]>, DbErr> {
        most_recent(&self.conn).await
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
                Event::Delegation(d) => delegation::process(&self.root, &tx, d).await?,
                Event::Invocation(i) => invocation::process(&self.root, &tx, i).await?,
                Event::Revocation(r) => revocation::process(&self.root, &tx, r).await?,
            });
        }
        // TODO update epoch table
        let seq = max_seq(&tx).await? + 1;
        let parents = most_recent(&tx).await?;

        tx.commit().await?;
        todo!()
    }

    pub async fn delegate(&self, delegation: Delegation) -> Result<Commit, TxError> {
        self.transact(vec![Event::Delegation(delegation)]).await
    }

    async fn invoke(&self, invocation: Invocation) -> Result<Commit, TxError> {
        self.transact(vec![Event::Invocation(invocation)]).await
    }

    async fn revoke_tx(&self, revocation: Revocation) -> Result<Commit, TxError> {
        self.transact(vec![Event::Revocation(revocation)]).await
    }

    // to allow users to make custom read queries
    async fn readable(&self) -> Result<DatabaseTransaction, DbErr> {
        self.conn
            .begin_with_config(None, Some(sea_orm::AccessMode::ReadOnly))
            .await
    }
}

async fn max_seq<C: ConnectionTrait>(db: &C) -> Result<u64, DbErr> {
    Ok(epoch::Entity::find()
        .select_only()
        .column_as(epoch::Column::Seq.max(), "seq")
        .into_tuple()
        .one(db)
        .await?
        .unwrap_or(0))
}

async fn most_recent<C: ConnectionTrait>(db: &C) -> Result<Vec<[u8; 32]>, DbErr> {
    Ok(todo!("get unconsumed latest tx from db"))
}

impl From<delegation::Error> for TxError {
    fn from(e: delegation::Error) -> Self {
        match e {
            delegation::Error::InvalidDelegation(e) => Self::InvalidDelegation(e),
            delegation::Error::Db(e) => Self::Db(e),
        }
    }
}

impl From<invocation::Error> for TxError {
    fn from(e: invocation::Error) -> Self {
        match e {
            invocation::Error::InvalidInvocation(e) => Self::InvalidInvocation(e),
            invocation::Error::Db(e) => Self::Db(e),
        }
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
