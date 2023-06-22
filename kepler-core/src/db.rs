use crate::events::{epoch_hash, Delegation, Event, HashError, Invocation, Operation, Revocation};
use crate::hash::Hash;
use crate::migrations::Migrator;
use crate::models::*;
use crate::relationships::*;
use crate::storage::{
    either::EitherError, Content, ImmutableDeleteStore, ImmutableReadStore, ImmutableStaging,
    ImmutableWriteStore,
};
use crate::types::{Metadata, OrbitIdWrap};
use crate::util::{Capability, DelegationInfo};
use futures::io::AsyncRead;
use kepler_lib::{
    authorization::{EncodingError, KeplerDelegation},
    resource::{KRIParseError, OrbitId},
};
use sea_orm::{
    entity::prelude::*, error::DbErr, query::*, ConnectionTrait, DatabaseTransaction,
    TransactionTrait,
};
use sea_orm_migration::MigratorTrait;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct OrbitDatabase<C, B, S> {
    conn: C,
    storage: B,
    staging: S,
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
    #[error(transparent)]
    ParseError(#[from] KRIParseError),
}

#[derive(Debug, thiserror::Error)]
pub enum TxStoreError<B, S>
where
    B: ImmutableReadStore + ImmutableWriteStore<S> + ImmutableDeleteStore,
    S: ImmutableStaging,
    S::Writable: 'static + Unpin,
{
    #[error(transparent)]
    Tx(#[from] TxError),
    #[error(transparent)]
    StoreRead(<B as ImmutableReadStore>::Error),
    #[error(transparent)]
    StoreWrite(<B as ImmutableWriteStore<S>>::Error),
    #[error(transparent)]
    StoreDelete(<B as ImmutableDeleteStore>::Error),
    #[error(transparent)]
    Staging(<S as ImmutableStaging>::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Missing Input for requested action")]
    MissingInput,
}

impl<B, S> From<DbErr> for TxStoreError<B, S>
where
    B: ImmutableReadStore + ImmutableWriteStore<S> + ImmutableDeleteStore,
    S: ImmutableStaging,
    S::Writable: 'static + Unpin,
{
    fn from(e: DbErr) -> Self {
        TxStoreError::Tx(e.into())
    }
}

impl<B, S> OrbitDatabase<DatabaseConnection, B, S> {
    pub async fn wrap(conn: DatabaseConnection, storage: B, staging: S) -> Result<Self, DbErr> {
        Migrator::up(&conn, None).await?;
        Ok(Self {
            conn,
            storage,
            staging,
        })
    }
}

impl<C, B, S> OrbitDatabase<C, B, S>
where
    C: TransactionTrait,
    B: ImmutableWriteStore<S> + ImmutableDeleteStore + ImmutableReadStore,
    S: ImmutableStaging,
    S::Writable: 'static + Unpin,
{
    async fn transact(&self, events: Vec<Event>) -> Result<HashMap<OrbitId, Commit>, TxError> {
        let tx = self
            .conn
            .begin_with_config(Some(sea_orm::IsolationLevel::ReadUncommitted), None)
            .await?;

        let commit = transact(&tx, events).await?;

        tx.commit().await?;

        Ok(commit)
    }

    pub async fn delegate(
        &self,
        delegation: Delegation,
    ) -> Result<HashMap<OrbitId, Commit>, TxError> {
        self.transact(vec![Event::Delegation(Box::new(delegation))])
            .await
    }

    pub async fn invoke<R>(
        &self,
        invocation: Invocation,
        mut inputs: HashMap<(OrbitId, String), (Metadata, R)>,
    ) -> Result<
        (
            HashMap<OrbitId, Commit>,
            Vec<InvocationOutcome<B::Readable>>,
        ),
        TxStoreError<B, S>,
    >
    where
        R: AsyncRead + Unpin,
    {
        let mut stages = HashMap::new();
        let mut ops = Vec::new();
        // for each capability being invoked
        for cap in invocation.0.capabilities.iter() {
            match cap
                .resource
                .kepler_resource()
                .and_then(|r| Some((r.service()?, cap.action.as_str(), r.orbit(), r.path()?)))
            {
                // stage inputs for content writes
                Some(("kv", "put", orbit, path)) => {
                    let (metadata, mut reader) = inputs
                        .remove(&(orbit.clone(), path.to_string()))
                        .ok_or(TxStoreError::MissingInput)?;
                    let mut stage = self
                        .staging
                        .stage(&orbit)
                        .await
                        .map_err(TxStoreError::Staging)?;

                    futures::io::copy(&mut reader, &mut stage).await?;
                    let value = stage.hash();

                    stages.insert((orbit.clone(), path.to_string()), stage);
                    // add write for tx
                    ops.push(Operation::KvWrite {
                        orbit: orbit.clone(),
                        key: path.to_string(),
                        metadata,
                        value,
                    });
                }
                // add delete for tx
                Some(("kv", "delete", orbit, path)) => {
                    ops.push(Operation::KvDelete {
                        orbit: orbit.clone(),
                        key: path.to_string(),
                        version: None,
                    });
                }
                _ => {}
            }
        }

        let tx = self
            .conn
            .begin_with_config(Some(sea_orm::IsolationLevel::ReadUncommitted), None)
            .await?;
        let caps = invocation.0.capabilities.clone();
        //  verify and commit invocation and kv operations
        let commit = transact(&tx, vec![Event::Invocation(Box::new(invocation), ops)]).await?;

        let mut results = Vec::new();
        // perform and record side effects
        for cap in caps {
            match (
                cap.resource
                    .kepler_resource()
                    .and_then(|r| Some((r.orbit(), r.service()?, r.path()?))),
                cap.action.as_str(),
            ) {
                (Some((orbit, "kv", path)), "get") => results.push(InvocationOutcome::KvRead(
                    get_kv(&tx, &self.storage, &orbit, &path)
                        .await
                        .map_err(|e| match e {
                            EitherError::A(e) => TxStoreError::Tx(e.into()),
                            EitherError::B(e) => TxStoreError::StoreRead(e),
                        })?,
                )),
                (Some((orbit, "kv", path)), "list") => {
                    results.push(InvocationOutcome::KvList(list(&tx, &orbit, &path).await?))
                }
                (Some((orbit, "kv", path)), "delete") => {
                    let kv = get_kv_entity(&tx, &orbit, &path).await?;
                    if let Some(kv) = kv {
                        self.storage
                            .remove(&orbit, &kv.value)
                            .await
                            .map_err(TxStoreError::StoreDelete)?;
                    }
                    results.push(InvocationOutcome::KvDelete)
                }
                (Some((orbit, "kv", path)), "put") => {
                    if let Some(stage) = stages.remove(&(orbit.clone(), path.to_string())) {
                        self.storage
                            .persist(&orbit, stage)
                            .await
                            .map_err(TxStoreError::StoreWrite)?;
                        results.push(InvocationOutcome::KvWrite)
                    }
                }
                (Some((orbit, "kv", path)), "metadata") => results.push(
                    InvocationOutcome::KvMetadata(metadata(&tx, &orbit, &path).await?),
                ),
                (Some((orbit, "capabilities", "")), "read") => results.push(
                    InvocationOutcome::OpenSessions(get_valid_delegations(&tx, &orbit).await?),
                ),
                _ => {}
            }
        }

        // commit tx if all side effects worked
        tx.commit().await?;
        Ok((commit, results))
    }

    pub async fn revoke(
        &self,
        revocation: Revocation,
    ) -> Result<HashMap<OrbitId, Commit>, TxError> {
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

pub enum InvocationOutcome<R> {
    KvList(Vec<String>),
    KvDelete,
    KvMetadata(Option<Metadata>),
    KvWrite,
    KvRead(Option<(Metadata, Content<R>)>),
    OpenSessions(HashMap<Hash, DelegationInfo>),
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

async fn event_orbits<'a, C: ConnectionTrait>(
    db: &C,
    ev: &'a [(Hash, Event)],
) -> Result<HashMap<OrbitId, Vec<&'a (Hash, Event)>>, DbErr> {
    // get orderings of events listed as revoked by events in the ev list
    let mut orbits = HashMap::<OrbitId, Vec<&'a (Hash, Event)>>::new();
    let revoked_events = event_order::Entity::find()
        .filter(
            event_order::Column::Event.is_in(ev.iter().filter_map(|(_, e)| match e {
                Event::Revocation(r) => Some(Hash::from(r.0.revoked)),
                _ => None,
            })),
        )
        .all(db)
        .await?;
    for e in ev {
        match &e.1 {
            Event::Delegation(d) => {
                for orbit in d.0.orbits() {
                    let entry = orbits.entry(orbit.clone()).or_insert_with(Vec::new);
                    if entry.iter().find(|(h, _)| h == &e.0).is_none() {
                        entry.push(e);
                    }
                }
            }
            Event::Invocation(i, _) => {
                for orbit in i.0.orbits() {
                    let entry = orbits.entry(orbit.clone()).or_insert_with(Vec::new);
                    if entry.iter().find(|(h, _)| h == &e.0).is_none() {
                        entry.push(e);
                    }
                }
            }
            Event::Revocation(r) => {
                let r_hash = Hash::from(r.0.revoked);
                for revoked in &revoked_events {
                    if r_hash == revoked.event {
                        let entry = orbits
                            .entry(revoked.orbit.0.clone())
                            .or_insert_with(Vec::new);
                        if entry.iter().find(|(h, _)| h == &e.0).is_none() {
                            entry.push(e);
                        }
                    }
                }
            }
        }
    }
    Ok(orbits)
}

pub(crate) async fn transact<C: ConnectionTrait>(
    db: &C,
    events: Vec<Event>,
) -> Result<HashMap<OrbitId, Commit>, TxError> {
    // for each event, get the hash and the relevent orbit(s)
    let event_hashes = events
        .into_iter()
        .map(|e| (e.hash(), e))
        .collect::<Vec<(Hash, Event)>>();
    let event_orbits = event_orbits(db, &event_hashes).await?;

    // get max sequence for each of the orbits
    let mut max_seqs = event_order::Entity::find()
        .filter(event_order::Column::Orbit.is_in(event_orbits.keys().cloned().map(OrbitIdWrap)))
        .group_by(event_order::Column::Orbit)
        .column_as(event_order::Column::Seq.max(), "max_seq")
        .all(db)
        .await?
        .into_iter()
        .fold(HashMap::new(), |mut m, order| {
            m.insert(order.orbit, order.seq + 1);
            m
        });

    // get 'most recent' epochs for each of the orbits
    let mut most_recent = epoch::Entity::find()
        .filter(epoch::Column::Orbit.is_in(event_orbits.keys().cloned().map(OrbitIdWrap)))
        .left_join(epoch_order::Entity)
        .column_as(epoch_order::Column::Child.is_null(), "r0")
        .all(db)
        .await?
        .into_iter()
        .fold(HashMap::new(), |mut m, epoch| {
            m.entry(epoch.orbit).or_insert_with(Vec::new).push(epoch.id);
            m
        });

    // get all the orderings and associated data
    let (epoch_order, orbit_order, event_order) = event_orbits
        .into_iter()
        .map(|(orbit, events)| {
            let parents = most_recent.remove(&orbit).unwrap_or_else(Vec::new);
            let epoch = epoch_hash(&orbit, &events, &parents)?;
            let seq = max_seqs.remove(&orbit).unwrap_or(0);
            Ok((orbit, (epoch, events, seq, parents)))
        })
        .collect::<Result<HashMap<_, _>, HashError>>()?
        .into_iter()
        .map(|(orbit, (epoch, hashes, seq, parents))| {
            (
                parents
                    .into_iter()
                    .map(|parent| epoch_order::Model {
                        parent,
                        child: epoch,
                        orbit: orbit.clone().into(),
                    })
                    .map(epoch_order::ActiveModel::from)
                    .collect::<Vec<epoch_order::ActiveModel>>(),
                (
                    orbit.clone(),
                    hashes
                        .iter()
                        .enumerate()
                        .map(|(i, (h, _))| (*h, (seq, epoch, i as i64)))
                        .collect::<HashMap<_, _>>(),
                ),
                hashes
                    .into_iter()
                    .enumerate()
                    .map(|(es, (hash, _))| event_order::Model {
                        event: *hash,
                        orbit: orbit.clone().into(),
                        seq,
                        epoch,
                        epoch_seq: es as i64,
                    })
                    .map(event_order::ActiveModel::from)
                    .collect::<Vec<event_order::ActiveModel>>(),
            )
        })
        .fold(
            (
                Vec::<epoch_order::ActiveModel>::new(),
                HashMap::<OrbitId, HashMap<Hash, (i64, Hash, i64)>>::new(),
                Vec::<event_order::ActiveModel>::new(),
            ),
            |(mut eo, mut oo, mut ev), (eo2, order, ev2)| {
                eo.extend(eo2);
                ev.extend(ev2);
                oo.insert(order.0, order.1);
                (eo, oo, ev)
            },
        );

    // save epoch orderings
    epoch_order::Entity::insert_many(epoch_order)
        .exec(db)
        .await?;

    // save event orderings
    event_order::Entity::insert_many(event_order)
        .exec(db)
        .await?;

    for (hash, event) in event_hashes {
        match event {
            Event::Delegation(d) => delegation::process(db, *d).await?,
            Event::Invocation(i, ops) => {
                invocation::process(
                    db,
                    *i,
                    ops.into_iter()
                        .map(|op| {
                            let v = orbit_order.get(op.orbit()).unwrap().get(&hash).unwrap();
                            op.version(v.0, v.1, v.2)
                        })
                        .collect(),
                )
                .await?
            }
            Event::Revocation(r) => revocation::process(db, *r).await?,
        };
    }

    todo!()
}

async fn list<C: ConnectionTrait>(
    db: &C,
    orbit: &OrbitId,
    prefix: &str,
) -> Result<Vec<String>, DbErr> {
    // get content id for key from db
    let mut list = kv_write::Entity::find()
        .filter(
            Condition::all()
                .add(kv_write::Column::Key.starts_with(prefix))
                .add(kv_write::Column::Orbit.eq(OrbitIdWrap(orbit.clone()))),
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

async fn metadata<C: ConnectionTrait>(
    db: &C,
    orbit: &OrbitId,
    key: &str,
    // version: Option<(i64, Hash, i64)>,
) -> Result<Option<Metadata>, DbErr> {
    match get_kv_entity(db, orbit, key).await? {
        Some(entry) => Ok(Some(entry.metadata)),
        None => Ok(None),
    }
}

async fn get_kv<C: ConnectionTrait, B: ImmutableReadStore>(
    db: &C,
    store: &B,
    orbit: &OrbitId,
    key: &str,
    // version: Option<(i64, Hash, i64)>,
) -> Result<Option<(Metadata, Content<B::Readable>)>, EitherError<DbErr, B::Error>> {
    let e = match get_kv_entity(db, orbit, key)
        .await
        .map_err(EitherError::A)?
    {
        Some(entry) => entry,
        None => return Ok(None),
    };
    let c = match store.read(orbit, &e.value).await.map_err(EitherError::B)? {
        Some(c) => c,
        None => return Ok(None),
    };
    Ok(Some((e.metadata, c)))
}

async fn get_kv_entity<C: ConnectionTrait>(
    db: &C,
    orbit: &OrbitId,
    key: &str,
    // version: Option<(i64, Hash, i64)>,
) -> Result<Option<kv_write::Model>, DbErr> {
    // Ok(if let Some((seq, epoch, epoch_seq)) = version {
    //     event_order::Entity::find_by_id((epoch, epoch_seq, orbit.clone().into()))
    //         .reverse_join(kv_write::Entity)
    //         .find_also_related(kv_delete::Entity)
    //         .filter(
    //             Condition::all()
    //                 .add(kv_write::Column::Key.eq(key))
    //                 .add(kv_write::Column::Orbit.eq(orbit.clone().into()))
    //                 .add(kv_delete::Column::InvocationId.is_null()),
    //         )
    //         .one(db)
    //         .await?
    //         .map(|(kv, _)| kv)
    // } else {
    // we want to find the latest kv_write which is not deleted
    Ok(kv_write::Entity::find()
        .filter(
            Condition::all()
                .add(kv_write::Column::Key.eq(key))
                .add(kv_write::Column::Orbit.eq(OrbitIdWrap(orbit.clone()))),
        )
        .order_by_desc(kv_write::Column::Seq)
        .order_by_desc(kv_write::Column::Epoch)
        .order_by_desc(kv_write::Column::EpochSeq)
        .find_also_related(kv_delete::Entity)
        .filter(kv_delete::Column::InvocationId.is_null())
        .one(db)
        .await?
        .map(|(kv, _)| kv))
    // })
}

async fn get_valid_delegations<C: ConnectionTrait>(
    db: &C,
    orbit: &OrbitId,
) -> Result<HashMap<Hash, DelegationInfo>, TxError> {
    let (dels, abilities): (Vec<delegation::Model>, Vec<Vec<abilities::Model>>) =
        delegation::Entity::find()
            .left_join(revocation::Entity)
            .filter(revocation::Column::Id.is_null())
            .find_with_related(abilities::Entity)
            .all(db)
            .await?
            .into_iter()
            .unzip();
    let parents = dels.load_many(parent_delegations::Entity, db).await?;
    let now = time::OffsetDateTime::now_utc();
    Ok(dels
        .into_iter()
        .zip(abilities)
        .zip(parents)
        .filter_map(|((del, ability), parents)| {
            if del.expiry.map(|e| e > now).unwrap_or(true)
                && del.not_before.map(|n| n <= now).unwrap_or(true)
                && ability.iter().any(|a| a.resource.orbit() == Some(orbit))
            {
                Some(match KeplerDelegation::from_bytes(&del.serialization) {
                    Ok(delegation) => Ok((
                        del.id,
                        DelegationInfo {
                            delegator: del.delegator,
                            delegate: del.delegatee,
                            parents: parents.into_iter().map(|p| p.parent.to_cid(0x55)).collect(),
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
