use ipfs_embed::{DefaultParams, Event as SwarmEvent, GossipEvent, Ipfs, PeerId, SwarmEvents};
use libipld::cid::Cid;
use rocket::async_trait;
use rocket::futures::{
    future::{join, join_all, ready},
    Future, Stream, StreamExt,
};
use rocket::tokio;
use serde::{Deserialize, Serialize};

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

    pub fn get<N: AsRef<[u8]>>(name: N) -> Result<Option<Cid>, anyhow::Error> {
        // resolve name
        // get from local storage
        Ok(None)
    }

    pub fn set<N: AsRef<[u8]>>(name: N, value: Cid) -> Result<(), anyhow::Error> {
        // make op
        // set local
        // start broadcast
        // return
        Ok(())
    }

    pub fn remove<N: AsRef<[u8]>>(name: N) -> Result<(), anyhow::Error> {
        // make op
        // set local
        // start broadcast
        // return
        Ok(())
    }
}

#[derive(Clone)]
pub struct KNSStore {
    pub id: String,
    pub ipfs: Ipfs<DefaultParams>,
}

impl KNSStore {
    pub fn new(id: String, ipfs: Ipfs<DefaultParams>) -> Self {
        Self { id, ipfs }
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

    async fn create_swarm(path: std::path::PathBuf) -> Result<Ipfs<DefaultParams>, anyhow::Error> {
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
        Ok(ipfs)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test() -> Result<(), anyhow::Error> {
        tracing_try_init();
        let tmp = tempdir::TempDir::new("test_streams")?;
        let db = sled::open(tmp.path().join("db"))?;

        let alice = create_swarm(tmp.path().join("alice")).await?;
        let bob = create_swarm(tmp.path().join("bob")).await?;
        let id = "test_id".to_string();
        std::thread::sleep_ms(2000);
        tracing::debug!("{:#?}", alice.peers());

        let alice_service =
            KeplerNameService::start(KNSStore::new(id.clone(), alice, db.open_tree("alice")?))
                .await?;
        let bob_service =
            KeplerNameService::start(KNSStore::new(id, bob, db.open_tree("bob")?)).await?;

        std::thread::sleep_ms(2000);

        assert!(false);
        Ok(())
    }
}
