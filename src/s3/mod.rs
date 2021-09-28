use anyhow::Result;
use ipfs_embed::{GossipEvent, PeerId};
use libipld::{cbor::DagCborCodec, cid::Cid, codec::Encode, multihash::Code, raw::RawCodec};
use rocket::futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

mod entries;
mod store;

pub use entries::{Object, ObjectBuilder};
pub use store::Store;

type Ipfs = ipfs_embed::Ipfs<ipfs_embed::DefaultParams>;
type Block = ipfs_embed::Block<ipfs_embed::DefaultParams>;
type TaskHandle = tokio::task::JoinHandle<()>;

pub struct Service {
    store: Store,
    task: TaskHandle,
}

impl Service {
    pub(crate) fn new(store: Store, task: TaskHandle) -> Self {
        Self { store, task }
    }

    pub fn start(config: Store) -> Result<Self> {
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
        Ok(Service::new(
            config.clone(),
            tokio::spawn(kv_task(events, config)),
        ))
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl std::ops::Deref for Service {
    type Target = Store;
    fn deref(&self) -> &Self::Target {
        &self.store
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

fn to_block<T: Encode<DagCborCodec>>(data: &T) -> Result<Block> {
    Ok(Block::encode(DagCborCodec, Code::Blake3_256, data)?)
}

fn to_block_raw<T: Encode<RawCodec>>(data: &T) -> Result<Block> {
    Ok(Block::encode(RawCodec, Code::Blake3_256, data)?)
}

#[derive(Serialize, Deserialize, Debug)]
enum KVMessage {
    Heads(#[serde(with = "vec_cid_bin")] Vec<Cid>),
    StateReq,
}

async fn kv_task(events: impl Stream<Item = Result<(PeerId, KVMessage)>> + Send, store: Store) {
    debug!("starting KV task");
    events
        .for_each_concurrent(None, |ev| async {
            match ev {
                Ok((p, KVMessage::Heads(heads))) => {
                    debug!("new heads from {}", p);
                    // sync heads
                    &store.try_merge_heads(heads.into_iter()).await;
                }
                Ok((p, KVMessage::StateReq)) => {
                    debug!("{} requests state", p);
                    // send heads
                    &store.broadcast_heads();
                }
                Err(e) => {
                    error!("{}", e);
                }
            }
        })
        .await;
}

#[cfg(test)]
mod test {
    use super::*;
    use ipfs_embed::{generate_keypair, Config, Event as SwarmEvent, Ipfs};
    use rocket::futures::StreamExt;
    use std::{collections::BTreeMap, time::Duration};
    use crate::tracing_try_init;

    async fn create_store(id: &str, path: std::path::PathBuf) -> Result<Store, anyhow::Error> {
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
        Store::new(id.to_string(), ipfs, sled::open(path.join("db.sled"))?)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test() -> Result<(), anyhow::Error> {
        tracing_try_init();
        let tmp = tempdir::TempDir::new("test_streams")?;
        let id = "test_id".to_string();

        let alice = create_store(&id, tmp.path().join("alice")).await?;
        let bob = create_store(&id, tmp.path().join("bob")).await?;

        let alice_service = alice.start_service()?;
        let bob_service = bob.start_service()?;
        std::thread::sleep(Duration::from_millis(500));

        let json = r#"{"hello":"there"}"#;
        let key1 = "my_json.json";
        let key2 = "my_dup_json.json";
        let md: BTreeMap<String, String> =
            [("content-type".to_string(), "application/json".to_string())]
                .to_vec()
                .into_iter()
                .collect();

        let s3_obj_1 = ObjectBuilder::new(key1.as_bytes().to_vec(), md.clone());
        let s3_obj_2 = ObjectBuilder::new(key2.as_bytes().to_vec(), md.clone());

        alice_service.write(vec![(s3_obj_1, json.as_bytes().to_vec())], vec![])?;
        bob_service.write(vec![(s3_obj_2, json.as_bytes().to_vec())], vec![])?;

        {
            // ensure only alice has s3_obj_1
            let o = alice_service
                .get(key1)?
                .expect("object 1 not found for alice");
            assert_eq!(&o.key, key1.as_bytes());
            assert_eq!(&o.metadata, &md);
            assert_eq!(bob_service.get(key1)?, None);
        };
        {
            // ensure only bob has s3_obj_2
            let o = bob_service.get(key2)?.expect("object 2 not found for bob");
            assert_eq!(&o.key, key2.as_bytes());
            assert_eq!(&o.metadata, &md);
            assert_eq!(alice_service.get(key2)?, None);
        };

        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(
            bob_service.get(key1)?.expect("object 1 not found for bob"),
            alice_service
                .get(key1)?
                .expect("object 1 not found for alice")
        );
        assert_eq!(
            bob_service.get(key2)?.expect("object 2 not found for bob"),
            alice_service
                .get(key2)?
                .expect("object 2 not found for alice")
        );

        // remove key1
        alice_service.write(vec![], vec![(key1.as_bytes().to_vec(), None)])?;

        assert_eq!(alice_service.get(key1)?, None);

        std::thread::sleep(Duration::from_millis(500));

        assert_eq!(bob_service.get(key1)?, None);

        Ok(())
    }
}
