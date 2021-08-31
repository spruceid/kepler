use anyhow::Result;
use ipfs_embed::{Event as SwarmEvent, GossipEvent, PeerId, SwarmEvents};
use libipld::cid::Cid;
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

pub trait Element {
    type Value;
    fn key(&self) -> &[u8];
    fn value(&self) -> Self::Value;
}

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

pub struct LinkedDelta {
    // previous heads
    prev: Vec<Cid>,
    delta: Delta,
}

impl LinkedDelta {
    pub fn new(prev: Vec<Cid>, delta: Delta) -> Self {
        Self { prev, delta }
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

    pub fn set(&self, heads: impl Iterator<Item = (Cid, u64)>) -> Result<()> {
        let mut batch = Batch::default();
        for (head, height) in heads {
            if !self.heads.contains_key(head.to_bytes())? {
                batch.insert(head.to_bytes(), &height.to_be_bytes());
            }
        }
        self.heads.apply_batch(batch)?;
        Ok(())
    }

    pub fn obsolete(&self, heads: impl Iterator<Item = Cid>) -> Result<()> {
        let mut batch = Batch::default();
        heads.for_each(|h| batch.remove(h.to_bytes()));
        self.heads.apply_batch(batch)?;
        Ok(())
    }
}

impl KNSStore {
    pub fn new(id: String, ipfs: Ipfs, db: Db) -> Result<Self> {
        // map key to element
        let elements = db.open_tree("elements")?;
        // map key to element
        let tombs = db.open_tree("tombs")?;
        // map key to value (?)
        let values = db.open_tree("values")?;
        // map key to current max priority for key
        let priorities = db.open_tree("priorities")?;
        // map current DAG head cids to their priority
        let heads = Heads::new(db.open_tree("heads")?)?;
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
    pub fn get<N: AsRef<[u8]>>(&self, name: N) -> Result<Option<Cid>> {
        // get from local storage
        Ok(None)
    }

    pub fn make_delta(&self, add: Vec<Cid>, rmv: Vec<Cid>) -> Result<LinkedDelta> {
        let (heads, height) = self.heads.state()?;
        Ok(LinkedDelta {
            prev: heads,
            delta: Delta::new(height, add, rmv),
        })
    }

    pub async fn commit(&self, delta: &LinkedDelta) -> Result<()> {
        self.apply(delta).await?;
        self.broadcast_heads().await?;
        Ok(())
    }

    async fn broadcast_heads(&self) -> Result<()> {
        let (heads, _) = self.heads.state()?;
        self.ipfs
            .publish(&self.id, bincode::serialize(&KVMessage::Heads(heads))?)?;
        Ok(())
    }

    async fn apply(&self, delta: &LinkedDelta) -> Result<()> {
        // find redundant heads
        // remove them
        // add new head
        // update tables
        todo!()
    }

    pub(crate) async fn try_merge_heads(&self, heads: impl Iterator<Item = Cid>) -> Result<()> {
        try_join_all(heads.map(|head| async move {
            // fetch head block
            // check block is an event
            // dispatch ipfs::sync
            tracing::debug!("syncing head {}", head);
            match self.ipfs.sync(&head, self.ipfs.peers()).await {
                Ok(_) => {tracing::debug!("synced head {}", head); Ok(())},
                Err(e) => {
                    tracing::error!("failed sync head {} {}", head, e);
                    Err(anyhow!(e))
                },
            }
        }))
            .await?;
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
                    &store.broadcast_heads().await;
                }
                Err(e) => {
                    tracing::error!("{}", e);
                }
            }
        })
        .await;
}
