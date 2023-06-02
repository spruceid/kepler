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
    entity::prelude::*, error::DbErr, query::*, ConnectionTrait, DatabaseTransaction,
    TransactionTrait,
};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct OrbitDatabase<C> {
    conn: C,
    orbit: OrbitId,
    root: String,
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub rev: Hash,
    pub seq: i64,
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

impl<C> OrbitDatabase<C> {
    pub fn wrap(conn: C, orbit: OrbitId) -> Self {
        Self {
            conn,
            root: orbit.did(),
            orbit,
        }
    }
}

impl<C> OrbitDatabase<C>
where
    C: ConnectionTrait,
{
    async fn get_max_seq(&self) -> Result<i64, DbErr> {
        max_seq(&self.conn, &self.orbit.to_string()).await
    }

    async fn get_most_recent(&self) -> Result<Vec<Hash>, DbErr> {
        most_recent(&self.conn, &self.orbit.to_string()).await
    }

    pub async fn get_valid_delegations(&self) -> Result<HashMap<Hash, DelegationInfo>, TxError> {
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

impl<C> OrbitDatabase<C>
where
    C: TransactionTrait,
{
    async fn transact(&self, events: Vec<Event>) -> Result<Commit, TxError> {
        let tx = self
            .conn
            .begin_with_config(Some(sea_orm::IsolationLevel::ReadUncommitted), None)
            .await?;

        let commit = transact(&tx, &self.orbit, &self.root, events).await?;

        tx.commit().await?;

        Ok(commit)
    }

    pub async fn delegate(&self, delegation: Delegation) -> Result<Commit, TxError> {
        self.transact(vec![Event::Delegation(Box::new(delegation))])
            .await
    }

    pub async fn invoke(&self, invocation: Invocation) -> Result<Commit, TxError> {
        self.transact(vec![Event::Invocation(Box::new(invocation))])
            .await
    }

    pub async fn revoke(&self, revocation: Revocation) -> Result<Commit, TxError> {
        self.transact(vec![Event::Revocation(Box::new(revocation))])
            .await
    }

    // to allow users to make custom read queries
    pub async fn readable(&self) -> Result<DatabaseTransaction, DbErr> {
        self.conn
            .begin_with_config(None, Some(sea_orm::AccessMode::ReadOnly))
            .await
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

pub(crate) async fn transact<C: ConnectionTrait>(
    db: &C,
    orbit_id: &OrbitId,
    root: &str,
    events: Vec<Event>,
) -> Result<Commit, TxError> {
    let orbit = orbit_id.to_string();

    let seq = max_seq(db, &orbit).await? + 1;
    let parents = most_recent(db, &orbit).await?;

    let (epoch_id, event_ids) = epoch_hash(seq, &events, &parents)?;

    epoch::Entity::insert(epoch::ActiveModel::from(epoch::Model {
        id: epoch_id,
        seq,
        orbit: orbit.clone(),
    }))
    .exec(db)
    .await?;

    for parent in parents.iter() {
        epochs::Entity::insert(epochs::ActiveModel::from(epochs::Model {
            parent: *parent,
            child: epoch_id,
            orbit: orbit.clone(),
        }))
        .exec(db)
        .await?;
    }

    for (epoch_seq, event) in events.into_iter().enumerate() {
        match event {
            // dropping db rolls back changes, so fine to '?' here
            Event::Delegation(d) => {
                delegation::process(root, &orbit, db, *d, seq, epoch_id, epoch_seq as i64).await?
            }
            Event::Invocation(i) => {
                invocation::process(root, &orbit, db, *i, seq, epoch_id, epoch_seq as i64).await?
            }
            Event::Revocation(r) => {
                revocation::process(root, &orbit, db, *r, seq, epoch_id, epoch_seq as i64).await?
            }
        };
    }

    Ok(Commit {
        rev: epoch_id,
        seq,
        committed_events: event_ids,
        consumed_epochs: parents,
    })
}

async fn max_seq<C: ConnectionTrait>(db: &C, orbit_id: &str) -> Result<i64, DbErr> {
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

pub(crate) async fn list<C: ConnectionTrait>(
    db: &C,
    orbit: &str,
    prefix: &str,
) -> Result<Vec<String>, DbErr> {
    // get content id for key from db
    let mut list = kv_write::Entity::find()
        .filter(
            Condition::all()
                .add(kv_write::Column::Key.starts_with(prefix))
                .add(kv_write::Column::Orbit.eq(orbit)),
        )
        .order_by_desc(kv_write::Column::Seq)
        .order_by_desc(kv_write::Column::EpochId)
        .find_also_related(kv_delete::Entity)
        .filter(kv_delete::Column::InvocationId.is_null())
        .all(db)
        .await?
        .into_iter()
        .map(|(kv, _)| kv.key)
        .collect::<Vec<String>>();
    list.dedup();
    Ok(list)
}

pub(crate) async fn metadata<C: ConnectionTrait>(
    db: &C,
    orbit: &str,
    key: &str,
    version: Option<(i64, Hash)>,
) -> Result<Option<kv_write::Metadata>, DbErr> {
    match get_kv_entity(db, orbit, key, version).await? {
        Some(entry) => Ok(Some(entry.metadata)),
        None => Ok(None),
    }
}

pub(crate) async fn get_kv_entity<C: ConnectionTrait>(
    db: &C,
    orbit: &str,
    key: &str,
    version: Option<(i64, Hash)>,
) -> Result<Option<kv_write::Model>, DbErr> {
    Ok(if let Some((seq, epoch)) = version {
        kv_write::Entity::find_by_id((orbit.to_string(), seq, epoch))
            .find_also_related(kv_delete::Entity)
            .filter(kv_delete::Column::InvocationId.is_null())
            .one(db)
            .await?
            .map(|(kv, _)| kv)
    } else {
        kv_write::Entity::find()
            .filter(
                Condition::all()
                    .add(kv_write::Column::Key.eq(key))
                    .add(kv_write::Column::Orbit.eq(orbit)),
            )
            .order_by_desc(kv_write::Column::Seq)
            .order_by_desc(kv_write::Column::EpochId)
            .find_also_related(kv_delete::Entity)
            .filter(kv_delete::Column::InvocationId.is_null())
            .one(db)
            .await?
            .map(|(kv, _)| kv)
    })
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
