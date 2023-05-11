use super::migrations::Migrator;
use crate::events::{Delegation, Event, Invocation, Revocation};
use crate::models::*;
use kepler_lib::resource::OrbitId;
use sea_orm::{
    entity::prelude::*, error::DbErr, query::QuerySelect, ConnectOptions, ConnectionTrait,
    Database, DatabaseConnection, DatabaseTransaction, TransactionTrait,
};
use sea_orm_migration::MigratorTrait;

#[derive(Debug, Clone)]
pub struct OrbitDatabase {
    conn: DatabaseConnection,
    orbit: OrbitId,
    root: String,
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub rev: [u8; 32],
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
    #[error(transparent)]
    InvalidRevocation(#[from] revocation::RevocationError),
}

impl OrbitDatabase {
    pub async fn new<C: Into<ConnectOptions>>(options: C, orbit: OrbitId) -> Result<Self, DbErr> {
        let conn = Database::connect(options).await?;
        Migrator::up(&conn, None).await?;

        Ok(Self {
            conn,
            root: orbit.did(),
            orbit,
        })
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

    pub async fn invoke(&self, invocation: Invocation) -> Result<Commit, TxError> {
        self.transact(vec![Event::Invocation(invocation)]).await
    }

    pub async fn revoke_tx(&self, revocation: Revocation) -> Result<Commit, TxError> {
        self.transact(vec![Event::Revocation(revocation)]).await
    }

    // to allow users to make custom read queries
    pub async fn readable(&self) -> Result<DatabaseTransaction, DbErr> {
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

impl From<revocation::Error> for TxError {
    fn from(e: revocation::Error) -> Self {
        match e {
            revocation::Error::InvalidRevocation(e) => Self::InvalidRevocation(e),
            revocation::Error::Db(e) => Self::Db(e),
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
