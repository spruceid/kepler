use crate::events::{epoch_hash, Delegation, Event, HashError, Invocation, Operation, Revocation};
use crate::hash::Hash;
use crate::keys::{get_did_key, Secrets};
use crate::migrations::Migrator;
use crate::models::*;
use crate::relationships::*;
use crate::storage::{
    either::EitherError, Content, HashBuffer, ImmutableDeleteStore, ImmutableReadStore,
    ImmutableStaging, ImmutableWriteStore, StorageSetup, StoreSize,
};
use crate::types::{Metadata, OrbitIdWrap};
use kepler_lib::{
    authorization::{EncodingError, KeplerDelegation, Resources},
    resource::{OrbitId, ResourceId},
    ssi::ucan::capabilities::Ability,
};
use sea_orm::{
    entity::prelude::*,
    error::{DbErr, RuntimeErr, SqlxError},
    query::*,
    sea_query::OnConflict,
    ConnectionTrait, DatabaseTransaction, TransactionTrait,
};
use sea_orm_migration::MigratorTrait;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct OrbitDatabase<C, B, S> {
    conn: C,
    storage: B,
    secrets: S,
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub rev: Hash,
    pub seq: i64,
    pub committed_events: Vec<Hash>,
    pub consumed_epochs: Vec<Hash>,
}

#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum TxError<S: StorageSetup, K: Secrets> {
    #[error("database error: {0}")]
    Db(#[from] DbErr),
    #[error(transparent)]
    Ucan(#[from] ssi::ucan::Error),
    #[error(transparent)]
    Cacao(#[from] kepler_lib::cacaos::v2::common::Error),
    #[error(transparent)]
    InvalidDelegation(ValidationError),
    #[error(transparent)]
    InvalidInvocation(ValidationError),
    #[error(transparent)]
    InvalidRevocation(#[from] revocation::RevocationError),
    #[error("Epoch Hashing Err: {0}")]
    EpochHashingErr(#[from] HashError),
    #[error(transparent)]
    Encoding(#[from] EncodingError),
    #[error(transparent)]
    StoreSetup(S::Error),
    #[error(transparent)]
    Secrets(K::Error),
    #[error("Orbit not found")]
    OrbitNotFound,
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("Missing event for service: {0} {1} {2:?}")]
    MissingServiceEvent(ResourceId, String, Option<(i64, Hash, i64)>),
}

#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum TxStoreError<B, S, K>
where
    B: ImmutableReadStore + ImmutableWriteStore<S> + ImmutableDeleteStore + StorageSetup,
    S: ImmutableStaging,
    S::Writable: 'static + Unpin,
    K: Secrets,
{
    #[error(transparent)]
    Tx(#[from] TxError<B, K>),
    #[error(transparent)]
    StoreRead(<B as ImmutableReadStore>::Error),
    #[error(transparent)]
    StoreWrite(<B as ImmutableWriteStore<S>>::Error),
    #[error(transparent)]
    StoreDelete(<B as ImmutableDeleteStore>::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Missing Input for requested action")]
    MissingInput,
}

impl<B, S, K> From<DbErr> for TxStoreError<B, S, K>
where
    B: ImmutableReadStore + ImmutableWriteStore<S> + ImmutableDeleteStore + StorageSetup,
    S: ImmutableStaging,
    S::Writable: 'static + Unpin,
    K: Secrets,
{
    fn from(e: DbErr) -> Self {
        TxStoreError::Tx(e.into())
    }
}

impl<B, K> OrbitDatabase<DatabaseConnection, B, K> {
    pub async fn new(conn: DatabaseConnection, storage: B, secrets: K) -> Result<Self, DbErr> {
        Migrator::up(&conn, None).await?;
        Ok(Self {
            conn,
            storage,
            secrets,
        })
    }
}

impl<C, B, K> OrbitDatabase<C, B, K>
where
    K: Secrets,
{
    pub async fn stage_key(&self, orbit: &OrbitId) -> Result<String, K::Error> {
        self.secrets.stage_keypair(orbit).await.map(get_did_key)
    }
}

impl<C, B, K> OrbitDatabase<C, B, K>
where
    C: TransactionTrait,
{
    // to allow users to make custom read queries
    pub async fn readable(&self) -> Result<DatabaseTransaction, DbErr> {
        self.conn
            .begin_with_config(None, Some(sea_orm::AccessMode::ReadOnly))
            .await
    }
}

impl<C, B, K> OrbitDatabase<C, B, K>
where
    B: StoreSize,
{
    pub async fn store_size(&self, orbit: &OrbitId) -> Result<Option<u64>, B::Error> {
        self.storage.total_size(orbit).await
    }
}

impl<C, B, K> OrbitDatabase<C, B, K>
where
    C: TransactionTrait,
{
    pub async fn check_db_connection(&self) -> Result<(), DbErr> {
        // there's a `ping` method on the connection, but we can't access it from here
        // but starting a transaction should be enough to check the connection
        self.conn.begin().await.map(|_| ())
    }
}

pub type InvocationInputs<W> = HashMap<(OrbitId, String), (Metadata, HashBuffer<W>)>;

impl<C, B, K> OrbitDatabase<C, B, K>
where
    C: TransactionTrait,
    B: StorageSetup,
    K: Secrets,
{
    async fn transact(
        &self,
        events: Vec<Event>,
    ) -> Result<HashMap<OrbitId, Commit>, TxError<B, K>> {
        let tx = self
            .conn
            .begin_with_config(Some(sea_orm::IsolationLevel::ReadUncommitted), None)
            .await?;

        let commit = transact(&tx, &self.storage, &self.secrets, events).await?;

        tx.commit().await?;

        Ok(commit)
    }

    pub async fn delegate(
        &self,
        delegation: Delegation,
    ) -> Result<HashMap<OrbitId, Commit>, TxError<B, K>> {
        self.transact(vec![Event::Delegation(Box::new(delegation))])
            .await
    }

    pub async fn revoke(
        &self,
        revocation: Revocation,
    ) -> Result<HashMap<OrbitId, Commit>, TxError<B, K>> {
        self.transact(vec![Event::Revocation(Box::new(revocation))])
            .await
    }

    pub async fn invoke<S>(
        &self,
        invocation: Invocation,
        mut inputs: InvocationInputs<S::Writable>,
    ) -> Result<
        (
            HashMap<OrbitId, Commit>,
            Vec<InvocationOutcome<B::Readable>>,
        ),
        TxStoreError<B, S, K>,
    >
    where
        B: ImmutableWriteStore<S> + ImmutableDeleteStore + ImmutableReadStore,
        S: ImmutableStaging,
        S::Writable: 'static + Unpin,
    {
        let mut stages = HashMap::new();
        let mut ops = Vec::new();
        // for each capability being invoked
        let activity: HashMap<_, _> = Resources::<'_, ResourceId>::grants(&invocation.0)
            .map(|(r, a)| (r, a.clone()))
            .collect();
        for (resource, actions) in activity.iter() {
            for action in actions.keys() {
                match (
                    action.namespace().as_ref(),
                    action.name().as_ref(),
                    resource.service(),
                    resource.path(),
                ) {
                    // stage inputs for content writes
                    ("kv", "put", Some("kv"), Some(path)) => {
                        let (metadata, mut stage) = inputs
                            .remove(&(resource.orbit().clone(), path.to_string()))
                            .ok_or(TxStoreError::MissingInput)?;

                        let value = stage.hash();

                        let norm_path = normalize_path(path);

                        stages.insert((resource.orbit().clone(), norm_path.to_string()), stage);
                        // add write for tx
                        ops.push(Operation::KvWrite {
                            orbit: resource.orbit().clone(),
                            key: norm_path.to_string(),
                            metadata,
                            value,
                        });
                    }
                    // add delete for tx
                    ("kv", "del", Some("kv"), Some(path)) => {
                        ops.push(Operation::KvDelete {
                            orbit: resource.orbit().clone(),
                            key: normalize_path(path).to_string(),
                            version: None,
                        });
                    }
                    _ => {}
                }
            }
        }

        let tx = self
            .conn
            .begin_with_config(Some(sea_orm::IsolationLevel::ReadUncommitted), None)
            .await?;
        //  verify and commit invocation and kv operations
        let commit = transact(
            &tx,
            &self.storage,
            &self.secrets,
            vec![Event::Invocation(Box::new(invocation), ops)],
        )
        .await?;

        let mut results = Vec::new();
        // perform and record side effects
        for (resource, actions) in activity.iter() {
            for action in actions.keys() {
                match (
                    action.namespace().as_ref(),
                    action.name().as_ref(),
                    resource.service(),
                    resource.path(),
                ) {
                    ("kv", "get", Some("kv"), Some(path)) => {
                        results.push(InvocationOutcome::KvRead(
                            get_kv(&tx, &self.storage, resource.orbit(), path)
                                .await
                                .map_err(|e| match e {
                                    EitherError::A(e) => TxStoreError::Tx(e.into()),
                                    EitherError::B(e) => TxStoreError::StoreRead(e),
                                })?,
                        ))
                    }
                    ("kv", "list", Some("kv"), Some(path)) => results.push(
                        InvocationOutcome::KvList(list(&tx, resource.orbit(), path).await?),
                    ),
                    ("kv", "del", Some("kv"), Some(path)) => {
                        let kv = get_kv_entity(&tx, resource.orbit(), path).await?;
                        if let Some(kv) = kv {
                            self.storage
                                .remove(resource.orbit(), &kv.value)
                                .await
                                .map_err(TxStoreError::StoreDelete)?;
                        }
                        results.push(InvocationOutcome::KvDelete)
                    }
                    ("kv", "put", Some("kv"), Some(path)) => {
                        if let Some(stage) =
                            stages.remove(&(resource.orbit().clone(), path.to_string()))
                        {
                            self.storage
                                .persist(resource.orbit(), stage)
                                .await
                                .map_err(TxStoreError::StoreWrite)?;
                            results.push(InvocationOutcome::KvWrite)
                        }
                    }
                    ("kv", "metadata", Some("kv"), Some(path)) => results.push(
                        InvocationOutcome::KvMetadata(metadata(&tx, resource.orbit(), path).await?),
                    ),
                    ("kv", "read", Some("capabilities"), Some("all")) => {
                        results.push(InvocationOutcome::OpenSessions(
                            get_valid_delegations(&tx, resource.orbit(), None).await?,
                        ))
                    }
                    _ => {}
                }
            }
        }

        // commit tx if all side effects worked
        tx.commit().await?;
        Ok((commit, results))
    }
}

#[derive(Debug)]
pub enum InvocationOutcome<R> {
    KvList(Vec<String>),
    KvDelete,
    KvMetadata(Option<Metadata>),
    KvWrite,
    KvRead(Option<(Metadata, Content<R>)>),
    OpenSessions(HashMap<Hash, KeplerDelegation>),
}

impl<S: StorageSetup, K: Secrets> From<revocation::Error> for TxError<S, K> {
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
                // TODO Event::Revocation(r) => Some(Hash::from(r.0.revoke)),
                _ => None,
            })),
        )
        .all(db)
        .await?;
    for e in ev {
        match &e.1 {
            Event::Delegation(d) => {
                for orbit in d.0.resources().map(|r: ResourceId| r.into_inner().0) {
                    let entry = orbits.entry(orbit).or_insert_with(Vec::new);
                    if !entry.iter().any(|(h, _)| h == &e.0) {
                        entry.push(e);
                    }
                }
            }
            Event::Invocation(i, _) => {
                for orbit in i.0.resources().map(|r: ResourceId| r.into_inner().0) {
                    let entry = orbits.entry(orbit).or_insert_with(Vec::new);
                    if !entry.iter().any(|(h, _)| h == &e.0) {
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
                        if !entry.iter().any(|(h, _)| h == &e.0) {
                            entry.push(e);
                        }
                    }
                }
            }
        }
    }
    Ok(orbits)
}

pub(crate) async fn transact<C: ConnectionTrait, S: StorageSetup, K: Secrets>(
    db: &C,
    store_setup: &S,
    secrets: &K,
    events: Vec<Event>,
) -> Result<HashMap<OrbitId, Commit>, TxError<S, K>> {
    // for each event, get the hash and the relevent orbit(s)
    let event_hashes = events
        .into_iter()
        .map(|e| (e.hash(), e))
        .collect::<Vec<(Hash, Event)>>();
    let event_orbits = event_orbits(db, &event_hashes).await?;
    let host = Ability::new("kepler/host").unwrap();
    let mut new_orbits = event_hashes
        .iter()
        .filter_map(|(_, e)| match e {
            Event::Delegation(d) => Some(
                Resources::<'_, ResourceId>::grants(&d.0)
                    .filter_map(|(k, a)| match k.into_inner() {
                        (orbit, None, None, None) if a.contains_key(&host) => Some(orbit),
                        _ => None,
                    })
                    .map(OrbitIdWrap),
            ),
            _ => None,
        })
        .flatten()
        .collect::<Vec<OrbitIdWrap>>();
    new_orbits.dedup();

    if !new_orbits.is_empty() {
        match orbit::Entity::insert_many(
            new_orbits
                .iter()
                .cloned()
                .map(|id| orbit::Model { id })
                .map(orbit::ActiveModel::from),
        )
        .on_conflict(
            OnConflict::column(orbit::Column::Id)
                .do_nothing()
                .to_owned(),
        )
        .exec(db)
        .await
        {
            Err(DbErr::RecordNotInserted) => (),
            r => {
                r?;
            }
        };
    }

    // get max sequence for each of the orbits
    let mut max_seqs = event_order::Entity::find()
        .filter(event_order::Column::Orbit.is_in(event_orbits.keys().cloned().map(OrbitIdWrap)))
        .select_only()
        .column(event_order::Column::Orbit)
        .column_as(event_order::Column::Seq.max(), "max_seq")
        .group_by(event_order::Column::Orbit)
        .into_tuple::<(OrbitIdWrap, i64)>()
        .all(db)
        .await?
        .into_iter()
        .fold(HashMap::new(), |mut m, (orbit, seq)| {
            m.insert(orbit, seq + 1);
            m
        });

    // get 'most recent' epochs for each of the orbits
    let mut most_recent = epoch::Entity::find()
        .select_only()
        .left_join(epoch_order::Entity)
        .filter(
            Condition::all()
                .add(epoch::Column::Orbit.is_in(event_orbits.keys().cloned().map(OrbitIdWrap)))
                .add(epoch_order::Column::Child.is_null()),
        )
        .column(epoch::Column::Orbit)
        .column(epoch::Column::Id)
        .into_tuple::<(OrbitIdWrap, Hash)>()
        .all(db)
        .await?
        .into_iter()
        .fold(HashMap::new(), |mut m, (orbit, epoch)| {
            m.entry(orbit).or_insert_with(Vec::new).push(epoch);
            m
        });

    // get all the orderings and associated data
    let (epoch_order, orbit_order, event_order, epochs) = event_orbits
        .into_iter()
        .map(|(orbit, events)| {
            let parents = most_recent.remove(&orbit).unwrap_or_default();
            let epoch = epoch_hash(&orbit, &events, &parents)?;
            let seq = max_seqs.remove(&orbit).unwrap_or(0);
            Ok((orbit, (epoch, events, seq, parents)))
        })
        .collect::<Result<HashMap<_, _>, HashError>>()?
        .into_iter()
        .map(|(orbit, (epoch, hashes, seq, parents))| {
            (
                parents
                    .iter()
                    .map(|parent| epoch_order::Model {
                        parent: *parent,
                        child: epoch,
                        orbit: orbit.clone().into(),
                    })
                    .map(epoch_order::ActiveModel::from)
                    .collect::<Vec<epoch_order::ActiveModel>>(),
                (
                    orbit.clone(),
                    (
                        seq,
                        epoch,
                        parents,
                        hashes
                            .iter()
                            .enumerate()
                            .map(|(i, (h, _))| (*h, i as i64))
                            .collect::<HashMap<_, _>>(),
                    ),
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
                epoch::Model {
                    seq,
                    id: epoch,
                    orbit: orbit.into(),
                },
            )
        })
        .fold(
            (
                Vec::<epoch_order::ActiveModel>::new(),
                HashMap::<OrbitId, (i64, Hash, Vec<Hash>, HashMap<Hash, i64>)>::new(),
                Vec::<event_order::ActiveModel>::new(),
                Vec::<epoch::ActiveModel>::new(),
            ),
            |(mut eo, mut oo, mut ev, mut ep), (eo2, order, ev2, ep2)| {
                eo.extend(eo2);
                ev.extend(ev2);
                oo.insert(order.0, order.1);
                ep.push(ep2.into());
                (eo, oo, ev, ep)
            },
        );

    // save epochs
    epoch::Entity::insert_many(epochs)
        .exec(db)
        .await
        .map_err(|e| match e {
            DbErr::Exec(RuntimeErr::SqlxError(SqlxError::Database(_))) => TxError::OrbitNotFound,
            _ => e.into(),
        })?;

    // save epoch orderings
    if !epoch_order.is_empty() {
        epoch_order::Entity::insert_many(epoch_order)
            .exec(db)
            .await?;
    }

    // save event orderings
    event_order::Entity::insert_many(event_order)
        .exec(db)
        .await?;

    for (hash, event) in event_hashes {
        match event {
            Event::Delegation(d) => delegation::process(db, *d).await.map_err(|e| e.to_del())?,
            Event::Invocation(i, ops) => invocation::process(
                db,
                *i,
                ops.into_iter()
                    .map(|op| {
                        let v = orbit_order
                            .get(op.orbit())
                            .and_then(|(s, e, _, h)| Some((s, e, h.get(&hash)?)))
                            .unwrap();
                        op.version(*v.0, *v.1, *v.2)
                    })
                    .collect(),
            )
            .await
            .map_err(|e| e.to_inv())?,
            Event::Revocation(r) => revocation::process(db, *r).await?,
        };
    }

    for orbit in new_orbits {
        store_setup
            .create(&orbit.0)
            .await
            .map_err(TxError::StoreSetup)?;
        secrets
            .save_keypair(&orbit.0)
            .await
            .map_err(TxError::Secrets)?;
    }

    Ok(orbit_order
        .into_iter()
        .map(|(o, (seq, rev, consumed_epochs, h))| {
            (
                o,
                Commit {
                    seq,
                    rev,
                    consumed_epochs,
                    committed_events: h.keys().cloned().collect(),
                },
            )
        })
        .collect())
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
    // TODO version: Option<(i64, Hash, i64)>,
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
    // TODO version: Option<(i64, Hash, i64)>,
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
    // TODO version: Option<(i64, Hash, i64)>,
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

async fn get_valid_delegations<C: ConnectionTrait, S: StorageSetup, K: Secrets>(
    db: &C,
    orbit: &OrbitId,
    time: Option<time::OffsetDateTime>,
) -> Result<HashMap<Hash, KeplerDelegation>, TxError<S, K>> {
    let (dels, abilities): (Vec<delegation::Model>, Vec<Vec<abilities::Model>>) =
        delegation::Entity::find()
            .left_join(revocation::Entity)
            .filter(revocation::Column::Id.is_null())
            .find_with_related(abilities::Entity)
            .all(db)
            .await?
            .into_iter()
            .unzip();
    let now = time.unwrap_or_else(time::OffsetDateTime::now_utc);
    Ok(dels
        .into_iter()
        .zip(abilities)
        .filter_map(|(del, ability)| {
            if del.expiry.map(|e| e > now).unwrap_or(true)
                && del.not_before.map(|n| n <= now).unwrap_or(true)
                && ability
                    .iter()
                    .any(|a| a.resource.as_ref().orbit() == Some(orbit))
            {
                Some(match del.reser_cacao() {
                    Ok(delegation) => Ok((del.id, delegation.0)),
                    Err(e) => Err(e),
                })
            } else {
                None
            }
        })
        .collect::<Result<HashMap<Hash, KeplerDelegation>, EncodingError>>()?)
}

fn normalize_path(p: &str) -> &str {
    if p.starts_with('/') {
        p.get(1..).unwrap_or("")
    } else {
        p
    }
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
