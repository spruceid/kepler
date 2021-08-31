use ipfs_embed::{DefaultParams, Event as SwarmEvent, GossipEvent, Ipfs, PeerId, SwarmEvents};
use libipld::{cid::Cid, Link};
use rocket::async_trait;
use rocket::futures::{future::join_all, Stream, StreamExt};
use rocket::tokio;
use serde::{Deserialize, Serialize};
use sled::{Db, Tree};
use std::convert::{TryFrom, TryInto};

#[async_trait]
pub trait KeplerService {
    type Error;
    type Stopped;

    async fn start(config: Self::Stopped) -> Result<Self, Self::Error>
    where
        Self: Sized;
    async fn stop(self);
}

type TaskHandle = tokio::task::JoinHandle<()>;

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

struct Delta {
    // max depth
    priority: u64,
    add: Vec<Cid>,
    rmv: Vec<Cid>,
}

pub struct LinkedDelta {
    // previous heads
    prev: Vec<Cid>,
    delta: Delta,
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

#[derive(Clone)]
pub struct KNSStore {
    pub id: String,
    pub ipfs: Ipfs<DefaultParams>,
    elements: Tree,
    tombs: Tree,
    values: Tree,
    priorities: Tree,
    heads: Heads,
}

#[derive(Clone)]
struct Heads {
    store: Tree,
}

impl Heads {
    pub fn new(store: Tree) -> anyhow::Result<Self> {
        Ok(Self { store })
    }

    pub fn state(&self) -> anyhow::Result<(Vec<Cid>, u64)> {
        self.store.iter().try_fold(
            (vec![], 0),
            |(mut heads, max_height), r| -> anyhow::Result<(Vec<Cid>, u64)> {
                let (head, hb) = r?;
                let height = u64::from_be_bytes(hb[..].try_into()?);
                heads.push(head[..].try_into()?);
                Ok((heads, u64::max(max_height, height)))
            },
        )
    }

    pub fn get(&self, head: Cid) -> anyhow::Result<Option<u64>> {
        self.store
            .get(head.to_bytes())?
            .map(|h| Ok(u64::from_be_bytes(h[..].try_into()?)))
            .transpose()
    }

    pub fn set(&self, head: Cid, height: u64) -> anyhow::Result<()> {
        if !self.store.contains_key(head.to_bytes())? {
            self.store.insert(head.to_bytes(), &height.to_be_bytes())?;
        };
        Ok(())
    }

    pub fn clear(&self, head: Cid) -> anyhow::Result<()> {
        self.store.remove(head.to_bytes())?;
        Ok(())
    }
}

impl KNSStore {
    pub fn new(id: String, ipfs: Ipfs<DefaultParams>, db: Db) -> anyhow::Result<Self> {
        // map key to element CIDs
        let elements = db.open_tree("elements")?;
        // map key to element CIDs
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
    pub fn get<N: AsRef<[u8]>>(&self, name: N) -> Result<Option<Cid>, anyhow::Error> {
        // get from local storage
        Ok(None)
    }

    pub fn transact(&self, add: Vec<Cid>, rmv: Vec<Cid>) -> anyhow::Result<LinkedDelta> {
        let (heads, height) = self.heads.state()?;
        Ok(LinkedDelta {
            prev: heads,
            delta: Delta::new(height, add, rmv),
        })
    }

    pub async fn commit(&self, delta: &LinkedDelta) -> anyhow::Result<()> {
        self.apply(delta).await?;
        self.broadcast_heads().await?;
        Ok(())
    }

    async fn broadcast_heads(&self) -> anyhow::Result<()> {
        let (heads, _) = self.heads.state()?;
        self.ipfs
            .publish(&self.id, bincode::serialize(&KVMessage::Heads(heads))?)?;
        Ok(())
    }

    async fn apply(&self, delta: &LinkedDelta) -> anyhow::Result<()> {
        // find redundant heads
        // remove them
        // add new head
        // update tables
        todo!()
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

mod vec_cid_bin {
    use libipld::cid::Cid;
    use serde::{de::Error as DeError, ser::SerializeSeq, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(vec: &Vec<Cid>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = ser.serialize_seq(Some(vec.len()))?;
        for cid in vec {
            seq.serialize_element(&cid.to_bytes())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D>(deser: D) -> Result<Vec<Cid>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Vec<&[u8]> = Deserialize::deserialize(deser)?;
        s.iter()
            .map(|&sc| Cid::read_bytes(sc).map_err(D::Error::custom))
            .collect()
    }
}

#[derive(Serialize, Deserialize, Debug)]
enum KVMessage {
    Heads(#[serde(with = "vec_cid_bin")] Vec<Cid>),
    StateReq,
}

async fn kv_task(
    events: impl Stream<Item = anyhow::Result<(PeerId, KVMessage)>> + Send,
    store: KNSStore,
) {
    tracing::debug!("starting KV task");
    events
        .for_each_concurrent(None, |ev| async {
            match ev {
                Ok((p, KVMessage::Heads(heads))) => {
                    tracing::debug!("new heads from {}", p);
                    // sync heads
                    sync_task(heads, &store).await;
                }
                Ok((p, KVMessage::StateReq)) => {
                    tracing::debug!("{} requests state", p);
                    // send heads
                }
                Err(e) => {
                    tracing::error!("{}", e);
                }
            }
        })
        .await;
}

async fn sync_task(heads: Vec<Cid>, store: &KNSStore) {
    join_all(heads.into_iter().map(|head| async move {
        // fetch head block
        // check block is an event
        // dispatch ipfs::sync
        tracing::debug!("syncing head {}", head);
        match store.ipfs.sync(&head, store.ipfs.peers()).await {
            Ok(_) => tracing::debug!("synced head {}", head),
            Err(e) => tracing::debug!("failed sync head {} {}", head, e),
        };
    }))
    .await;
}

#[cfg(test)]
mod test {
    use super::*;
    use ipfs_embed::{generate_keypair, Config};
    fn tracing_try_init() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init()
            .ok();
    }

    async fn create_store(id: &str, path: std::path::PathBuf) -> Result<KNSStore, anyhow::Error> {
        std::fs::create_dir_all(&path)?;
        let mut config = Config::new(&path, generate_keypair());
        config.network.broadcast = None;
        let ipfs = Ipfs::new(config).await?;
        ipfs.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?.next().await;
        let task_ipfs = ipfs.clone();
        tokio::spawn(async move {
            let mut events = task_ipfs.swarm_events();
            loop {
                match events.next().await {
                    Some(SwarmEvent::Discovered(p)) => {
                        tracing::debug!("dialing peer {}", p);
                        &task_ipfs.dial(&p);
                    }
                    None => return,
                    _ => continue,
                }
            }
        });
        KNSStore::new(id.to_string(), ipfs, sled::open(path.join("db.sled"))?)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test() -> Result<(), anyhow::Error> {
        tracing_try_init();
        let tmp = tempdir::TempDir::new("test_streams")?;
        let id = "test_id".to_string();

        let alice = create_store(&id, tmp.path().join("alice")).await?;
        let bob = create_store(&id, tmp.path().join("bob")).await?;
        std::thread::sleep_ms(2000);

        let alice_service = KeplerNameService::start(alice).await?;
        let bob_service = KeplerNameService::start(bob).await?;

        std::thread::sleep_ms(2000);

        assert!(false);
        Ok(())
    }
}
