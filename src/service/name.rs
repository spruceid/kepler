use crate::service::s3::{S3Object, S3ObjectData};
use anyhow::Result;
use ipfs_embed::{Event as SwarmEvent, GossipEvent, PeerId, SwarmEvents};
use libipld::{
    cbor::DagCborCodec,
    cid::Cid,
    codec::{Codec, Decode, Encode},
    multihash::Code,
    raw::RawCodec,
    DagCbor,
};
use rocket::async_trait;
use rocket::futures::{
    future::{join_all, try_join_all},
    Stream, StreamExt,
};
use rocket::tokio;
use serde::{Deserialize, Serialize};
use sled::{Batch, Db, Tree};
use std::convert::{TryFrom, TryInto};

use super::{vec_cid_bin, KeplerService};

type TaskHandle = tokio::task::JoinHandle<()>;
type Ipfs = ipfs_embed::Ipfs<ipfs_embed::DefaultParams>;
type Block = ipfs_embed::Block<ipfs_embed::DefaultParams>;

pub fn to_block<T: Encode<DagCborCodec>>(data: &T) -> Result<Block> {
    Ok(Block::encode(DagCborCodec, Code::Blake3_256, data)?)
}

pub fn to_block_raw<T: Encode<RawCodec>>(data: &T) -> Result<Block> {
    Ok(Block::encode(RawCodec, Code::Blake3_256, data)?)
}

pub struct KeplerNameService {
    store: KNSStore,
    task: TaskHandle,
}

impl KeplerNameService {
    pub(crate) fn new(store: KNSStore, task: TaskHandle) -> Self {
        Self { store, task }
    }
}

impl std::ops::Deref for KeplerNameService {
    type Target = KNSStore;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

#[derive(DagCbor)]
pub struct Delta {
    // max depth
    priority: u64,
    add: Vec<Cid>,
    rmv: Vec<Cid>,
}

impl Delta {
    pub fn new(priority: u64, add: Vec<Cid>, rmv: Vec<Cid>) -> Self {
        Self { priority, add, rmv }
    }

    pub fn merge(self, other: Self) -> Self {
        let mut add = self.add;
        let mut other_add = other.add;
        add.append(&mut other_add);

        let mut rmv = self.rmv;
        let mut other_rmv = other.rmv;
        rmv.append(&mut other_rmv);

        Self {
            add,
            rmv,
            priority: u64::max(self.priority, other.priority),
        }
    }
}

#[derive(DagCbor)]
pub struct LinkedDelta {
    // previous heads
    prev: Vec<Cid>,
    delta: Delta,
}

impl LinkedDelta {
    pub fn new(prev: Vec<Cid>, delta: Delta) -> Self {
        Self { prev, delta }
    }

    pub fn to_block(&self) -> Result<Block> {
        to_block(self)
    }
}

#[derive(Clone)]
pub struct KNSStore {
    pub id: String,
    pub ipfs: Ipfs,
    elements: Tree,
    tombs: Tree,
    values: Tree,
    priorities: Tree,
    heads: Heads,
}

#[derive(Clone)]
pub struct Heads {
    heads: Tree,
}

impl Heads {
    pub fn new(db: Db) -> Result<Self> {
        Ok(Self {
            heads: db.open_tree("heads")?,
        })
    }

    pub fn state(&self) -> Result<(Vec<Cid>, u64)> {
        self.heads.iter().try_fold(
            (vec![], 0),
            |(mut heads, max_height), r| -> Result<(Vec<Cid>, u64)> {
                let (head, hb) = r?;
                let height = u64::from_be_bytes(hb[..].try_into()?);
                heads.push(head[..].try_into()?);
                Ok((heads, u64::max(max_height, height)))
            },
        )
    }

    pub fn get(&self, head: Cid) -> Result<Option<u64>> {
        self.heads
            .get(head.to_bytes())?
            .map(|h| Ok(u64::from_be_bytes(h[..].try_into()?)))
            .transpose()
    }

    pub fn set(&self, heads: impl IntoIterator<Item = (Cid, u64)>) -> Result<()> {
        let mut batch = Batch::default();
        for (head, height) in heads.into_iter() {
            if !self.heads.contains_key(head.to_bytes())? {
                tracing::debug!("setting head height {} {}", head, height);
                batch.insert(head.to_bytes(), &height.to_be_bytes());
            }
        }
        self.heads.apply_batch(batch)?;
        Ok(())
    }

    pub fn clear(&self, heads: impl IntoIterator<Item = Cid>) -> Result<()> {
        let mut batch = Batch::default();
        heads.into_iter().for_each(|h| batch.remove(h.to_bytes()));
        self.heads.apply_batch(batch)?;
        Ok(())
    }
}

impl KNSStore {
    pub fn new(id: String, ipfs: Ipfs, db: Db) -> Result<Self> {
        // map key to element cid
        let elements = db.open_tree("elements")?;
        // map key to element cid
        let tombs = db.open_tree("tombs")?;
        // map key to value (?)
        let values = db.open_tree("values")?;
        // map key to current max priority for key
        let priorities = db.open_tree("priorities")?;
        // map current DAG head cids to their priority
        let heads = Heads::new(db)?;
        Ok(Self {
            id,
            ipfs,
            elements,
            tombs,
            values,
            priorities,
            heads,
        })
    }
    pub fn get<N: AsRef<[u8]>>(&self, name: N) -> Result<Option<S3Object>> {
        let key = name;
        let cid = match (
            // check element isnt tombstoned
            !self.tombs.contains_key(&key)?,
            self.elements
                .get(&key)?
                .map(|b| Cid::try_from(b.as_ref()))
                .transpose()?,
        ) {
            (true, Some(cid)) => cid,
            _ => return Ok(None),
        };
        Ok(Some(self.ipfs.get(&cid)?.decode()?))
    }

    pub fn make_delta(
        &self,
        add: impl IntoIterator<Item = S3ObjectData>,
        rmv: impl IntoIterator<Item = (Vec<u8>, Option<String>)>,
    ) -> Result<LinkedDelta> {
        let (heads, height) = self.heads.state()?;
        let adds: Result<Vec<Cid>> = add
            .into_iter()
            .map(|d| {
                let d_block = to_block(&d)?;
                // get version string, height + data cid
                let version_id = format!("{}.{}", height, d_block.cid());
                let s3_block = S3Object::new(d, version_id).to_block()?;
                self.ipfs.insert(&s3_block)?;
                Ok(*s3_block.cid())
            })
            .collect();
        let rmvs: Result<Vec<Cid>> = rmv
            .into_iter()
            .map(|(key, version_id)| {
                // TODO find CID from version id and key
                Err(anyhow!("todo"))
            })
            .collect();
        Ok(LinkedDelta {
            prev: heads,
            delta: Delta::new(height, adds?, rmvs?),
        })
    }

    pub async fn commit(&self, delta: &LinkedDelta) -> Result<()> {
        tracing::debug!("committing delta with priority {}", delta.delta.priority);
        self.apply(delta).await?;
        self.broadcast_heads()?;
        Ok(())
    }

    fn broadcast_heads(&self) -> Result<()> {
        let (heads, height) = self.heads.state()?;
        if !heads.is_empty() {
            tracing::debug!("broadcasting {} heads at maxheight {}", heads.len(), height);
            self.ipfs
                .publish(&self.id, bincode::serialize(&KVMessage::Heads(heads))?)?;
        }
        Ok(())
    }

    async fn apply(&self, delta: &LinkedDelta) -> Result<()> {
        // ensure dont double add or remove
        // find redundant heads and remove them
        // add new head
        let block = delta.to_block()?;
        self.heads.set(vec![(*block.cid(), delta.delta.priority)])?;
        self.ipfs.insert(&block)?;
        // TODO update tables

        Ok(())
    }

    pub(crate) async fn try_merge_heads(&self, heads: impl Iterator<Item = Cid>) -> Result<()> {
        try_join_all(heads.map(|head| async move {
            // fetch head block
            // check block is an event
            // dispatch ipfs::sync
            tracing::debug!("syncing head {}", head);
            match self.ipfs.sync(&head, self.ipfs.peers()).await {
                Ok(_) => {
                    tracing::debug!("synced head {}", head);
                    Ok(())
                }
                Err(e) => {
                    tracing::error!("failed sync head {}", e);
                    Err(anyhow!(e))
                }
            }
        }))
        .await?;
        Ok(())
    }

    pub(crate) fn request_heads(&self) -> Result<()> {
        tracing::debug!("requesting heads");
        self.ipfs
            .publish(&self.id, bincode::serialize(&KVMessage::StateReq)?)?;
        Ok(())
    }
}

#[async_trait]
impl KeplerService for KeplerNameService {
    type Error = anyhow::Error;
    type Stopped = KNSStore;

    async fn start(config: Self::Stopped) -> Result<Self, Self::Error> {
        let events = config.ipfs.subscribe(&config.id)?.filter_map(|e| async {
            match e {
                GossipEvent::Message(p, d) => Some(match bincode::deserialize(&d) {
                    Ok(m) => Ok((p, m)),
                    Err(e) => Err(anyhow!(e)),
                }),
                _ => None,
            }
        });
        config.request_heads()?;
        Ok(KeplerNameService::new(
            config.clone(),
            tokio::spawn(kv_task(events, config)),
        ))
    }
    async fn stop(self) {
        self.task.abort();
    }
}

impl Drop for KeplerNameService {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Serialize, Deserialize, Debug)]
enum KVMessage {
    Heads(#[serde(with = "vec_cid_bin")] Vec<Cid>),
    StateReq,
}

async fn kv_task(events: impl Stream<Item = Result<(PeerId, KVMessage)>> + Send, store: KNSStore) {
    tracing::debug!("starting KV task");
    events
        .for_each_concurrent(None, |ev| async {
            match ev {
                Ok((p, KVMessage::Heads(heads))) => {
                    tracing::debug!("new heads from {}", p);
                    // sync heads
                    &store.try_merge_heads(heads.into_iter()).await;
                }
                Ok((p, KVMessage::StateReq)) => {
                    tracing::debug!("{} requests state", p);
                    // send heads
                    &store.broadcast_heads();
                }
                Err(e) => {
                    tracing::error!("{}", e);
                }
            }
        })
        .await;
}
