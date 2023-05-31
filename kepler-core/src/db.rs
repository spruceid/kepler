use super::migrations::Migrator;
use crate::events::{epoch_hash, Delegation, Event, HashError, Invocation, Revocation};
use crate::hash::Hash;
use crate::models::*;
use crate::relationships::*;
use crate::util::{Capability, DelegationInfo};
use kepler_lib::{
    authorization::{EncodingError, KeplerDelegation},
    resource::OrbitId,
};
use sea_orm::{
    entity::prelude::*, error::DbErr, query::QuerySelect, ConnectOptions, ConnectionTrait,
    Database, DatabaseConnection, DatabaseTransaction, TransactionTrait,
};
use sea_orm_migration::MigratorTrait;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct OrbitDatabase {
    conn: DatabaseConnection,
    orbit: OrbitId,
    root: String,
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub rev: Hash,
    pub seq: u32,
    pub committed_events: Vec<Hash>,
    pub consumed_epochs: Vec<Hash>,
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
    #[error("Epoch Hashing Err")]
    EpochHashingErr(#[from] HashError),
    #[error(transparent)]
    Encoding(#[from] EncodingError),
}

impl OrbitDatabase {
    pub async fn new<C: Into<ConnectOptions>>(options: C, orbit: OrbitId) -> Result<Self, DbErr> {
        let conn = Database::connect(options).await?;
        Migrator::up(&conn, None).await?;
        Self::wrap(conn, orbit).await
    }

    pub async fn wrap(conn: DatabaseConnection, orbit: OrbitId) -> Result<Self, DbErr> {
        Ok(Self {
            conn,
            root: orbit.did(),
            orbit,
        })
    }

    pub async fn get_max_seq(&self) -> Result<u32, DbErr> {
        max_seq(&self.conn, &self.orbit.to_string()).await
    }

    pub async fn get_most_recent(&self) -> Result<Vec<Hash>, DbErr> {
        most_recent(&self.conn, &self.orbit.to_string()).await
    }

    pub async fn transact(&self, events: Vec<Event>) -> Result<Commit, TxError> {
        let tx = self
            .conn
            .begin_with_config(Some(sea_orm::IsolationLevel::ReadUncommitted), None)
            .await?;
        let orbit = self.orbit.to_string();

        let seq = max_seq(&tx, &orbit).await? + 1;
        let parents = most_recent(&tx, &orbit).await?;

        let (epoch_id, event_ids) = epoch_hash(seq, &events, &parents)?;

        epoch::Entity::insert(epoch::ActiveModel::from(epoch::Model {
            id: epoch_id,
            seq,
            orbit: orbit.clone(),
        }))
        .exec(&tx)
        .await?;

        for parent in parents.iter() {
            epochs::Entity::insert(epochs::ActiveModel::from(epochs::Model {
                parent: *parent,
                child: epoch_id,
                orbit: orbit.clone(),
            }))
            .exec(&tx)
            .await?;
        }

        for (epoch_seq, event) in events.into_iter().enumerate() {
            match event {
                // dropping tx rolls back changes, so fine to '?' here
                Event::Delegation(d) => {
                    delegation::process(
                        &self.root,
                        &orbit,
                        &tx,
                        *d,
                        seq,
                        epoch_id,
                        epoch_seq as u32,
                    )
                    .await?
                }
                Event::Invocation(i) => {
                    invocation::process(
                        &self.root,
                        &orbit,
                        &tx,
                        *i,
                        seq,
                        epoch_id,
                        epoch_seq as u32,
                    )
                    .await?
                }
                Event::Revocation(r) => {
                    revocation::process(
                        &self.root,
                        &orbit,
                        &tx,
                        *r,
                        seq,
                        epoch_id,
                        epoch_seq as u32,
                    )
                    .await?
                }
            };
        }

        tx.commit().await?;

        Ok(Commit {
            rev: epoch_id,
            seq,
            committed_events: event_ids,
            consumed_epochs: parents,
        })
    }

    pub async fn delegate(&self, delegation: Delegation) -> Result<Commit, TxError> {
        self.transact(vec![Event::Delegation(Box::new(delegation))])
            .await
    }

    pub async fn invoke(&self, invocation: Invocation) -> Result<Commit, TxError> {
        self.transact(vec![Event::Invocation(Box::new(invocation))])
            .await
    }

    pub async fn revoke_tx(&self, revocation: Revocation) -> Result<Commit, TxError> {
        self.transact(vec![Event::Revocation(Box::new(revocation))])
            .await
    }

    // to allow users to make custom read queries
    pub async fn readable(&self) -> Result<DatabaseTransaction, DbErr> {
        self.conn
            .begin_with_config(None, Some(sea_orm::AccessMode::ReadOnly))
            .await
    }

    pub async fn get_valid_delegations(&self) -> Result<HashMap<Hash, DelegationInfo>, TxError> {
        use sea_orm::entity::prelude::*;
        let orbit = self.orbit.to_string();
        let (dels, abilities): (Vec<delegation::Model>, Vec<Vec<abilities::Model>>) =
            delegation::Entity::find()
                .filter(delegation::Column::Orbit.eq(&orbit))
                .left_join(revocation::Entity)
                .filter(revocation::Column::Id.is_null())
                .find_with_related(abilities::Entity)
                .all(&self.conn)
                .await?
                .into_iter()
                .unzip();
        let parents = dels
            .load_many(parent_delegations::Entity, &self.conn)
            .await?;
        let now = time::OffsetDateTime::now_utc();
        Ok(dels
            .into_iter()
            .zip(abilities)
            .zip(parents)
            .filter_map(|((del, ability), parents)| {
                if del.expiry.map(|e| e > now).unwrap_or(true)
                    && del.not_before.map(|n| n <= now).unwrap_or(true)
                {
                    Some(match KeplerDelegation::from_bytes(&del.serialization) {
                        Ok(delegation) => Ok((
                            del.id,
                            DelegationInfo {
                                delegator: del.delegator,
                                delegate: del.delegatee,
                                parents: parents
                                    .into_iter()
                                    .map(|p| p.parent.to_cid(0x55))
                                    .collect(),
                                expiry: del.expiry,
                                not_before: del.not_before,
                                issued_at: del.issued_at,
                                capabilities: ability
                                    .into_iter()
                                    .map(|a| Capability {
                                        resource: a.resource,
                                        action: a.ability,
                                    })
                                    .collect(),
                                delegation,
                            },
                        )),
                        Err(e) => Err(e),
                    })
                } else {
                    None
                }
            })
            .collect::<Result<HashMap<Hash, DelegationInfo>, EncodingError>>()?)
    }
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

async fn max_seq<C: ConnectionTrait>(db: &C, orbit_id: &str) -> Result<u32, DbErr> {
    Ok(epoch::Entity::find()
        .filter(epoch::Column::Orbit.eq(orbit_id))
        .select_only()
        .column_as(epoch::Column::Seq.max(), "max_seq")
        .into_tuple()
        .one(db)
        .await?
        // to account for if there are no epochs yet
        .unwrap_or(None)
        .unwrap_or(0))
}

async fn most_recent<C: ConnectionTrait>(db: &C, orbit_id: &str) -> Result<Vec<Hash>, DbErr> {
    use sea_orm::{entity::prelude::*, query::*};
    // find epochs which do not appear in the parent column of the parent_epochs junction table
    epoch::Entity::find()
        .filter(epoch::Column::Orbit.eq(orbit_id))
        .left_join(epochs::Entity)
        .filter(epochs::Column::Parent.is_null())
        .select_only()
        .column(epoch::Column::Id)
        .into_tuple()
        .all(db)
        .await
}

#[cfg(test)]
mod test {
    use super::*;
    use async_std::test;

    async fn get_db(o: OrbitId) -> Result<OrbitDatabase, DbErr> {
        OrbitDatabase::new("sqlite::memory:", o).await
    }

    #[test]
    async fn basic() {
        let db = get_db(OrbitId::new(
            "example:alice".to_string(),
            "default".to_string(),
        ))
        .await
        .unwrap();
    }
}
