use crate::events::{epoch_hash, Delegation, Event, HashError, Invocation, Revocation};
use crate::hash::{hash, Hash};
use crate::models::*;
use crate::relationships::*;
use crate::util::{Capability, DelegationInfo};
use kepler_lib::{
    authorization::{EncodingError, KeplerDelegation},
    resource::{KRIParseError, OrbitId},
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

        let commit = transact(&tx, events).await?;

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
    events: Vec<Event>,
) -> Result<HashMap<OrbitId, Commit>, TxError> {
    // for each event, get the hash and the relevent orbit(s)
    let (orbits, events) = events
        .into_iter()
        .map(|event| {
            let (hash, orbits) = match event {
                Event::Delegation(d) => (hash(&d.1), d.0.orbits()),
                Event::Invocation(i) => (hash(&i.1), i.0.orbits()),
                Event::Revocation(r) => (hash(&r.1), r.0.orbits()),
            };
            (hash, event, orbits)
        })
        .fold(
            (HashMap::new(), Vec::new()),
            |(mut o, mut events), (hash, event, orbits)| {
                for orbit in orbits {
                    o.entry(orbit).or_insert_with(Vec::new).push(hash);
                }
                events.push((hash, event));
            },
        );

    // get max sequence for each of the orbits
    let mut max_seqs = event_order::Entity::find()
        .filter(event_order::Column::Orbit.is_in(orbits.keys().map(|o| o.to_string())))
        .group_by(event_order::Column::Orbit)
        .max(event_order::Column::Seq)
        .all(db)
        .await?
        .into_iter()
        .fold(HashMap::new(), |mut m, (orbit, seq)| {
            m.insert(orbit, seq + 1);
            m
        });

    // get 'most recent' epochs for each of the orbits
    let mut most_recent = epoch_order::Entity::find()
        .filter(epoch_order::Column::Child.is_in(orbits.keys().map(|o| o.to_string())))
        .left_join(epoch_order::Relation::Parent.def())
        .all(db)
        .await?
        .into_iter()
        .fold(HashMap::new(), |mut m, (orbit, recent)| {
            m.insert(orbit, recent);
            m
        });

    // get all the orderings and associated data
    let (epoch_order, event_order) = orbits
        .into_iter()
        .map(|(orbit, event_hashes)| {
            (
                orbit,
                event_hashes,
                max_seqs.remove(&orbit).unwrap_or(0),
                most_recent.remove(&orbit).unwrap_or_else(Vec::new),
                // TODO get hash of epoch
                todo!(),
            )
        })
        .map(|(orbit, hashes, seq, parents, epoch)| {
            (
                parents.into_iter().map(|parent| epoch_order::Model {
                    parent,
                    child: epoch,
                    orbit: orbit.clone(),
                }),
                hashes
                    .into_iter()
                    .enumerate()
                    .map(|(epoch_seq, event)| event_order::Model {
                        event,
                        orbit: orbit.clone(),
                        seq,
                        epoch,
                        epoch_seq,
                    }),
            )
        })
        .unzip();

    // save epoch orderings
    epoch_order::Entity::insert_many(epoch_order.flatten().map(epoch_order::ActiveModel::from))
        .exec(db)
        .await?;

    // save event orderings
    event_order::Entity::insert_many(event_order.flatten().map(event_order::ActiveModel::from))
        .exec(db)
        .await?;

    for (hash, event) in events {
        match event {
            Event::Delegation(d) => delegation::process(db, *d).await?,
            Event::Invocation(i) => invocation::process(db, *i).await?,
            Event::Revocation(r) => revocation::process(db, *r).await?,
        }
    }

    todo!()
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
