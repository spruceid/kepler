use anyhow::Result;
use ipfs::PeerId;
use libipld::{cbor::DagCborCodec, cid::Cid, codec::Encode, multihash::Code, raw::RawCodec};
use rocket::futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub mod behaviour;
mod entries;
mod store;

use super::ipfs::{Block, Ipfs};

pub use entries::{Object, ObjectBuilder, ObjectReader};
pub use store::Store;

type TaskHandle = tokio::task::JoinHandle<()>;

#[derive(Clone)]
pub struct Service {
    pub store: Store,
    task: Arc<TaskHandle>,
}

impl Service {
    pub(crate) fn new(store: Store, task: TaskHandle) -> Self {
        Self {
            store,
            task: Arc::new(task),
        }
    }

    pub async fn start(config: Store) -> Result<Self> {
        let events = config
            .ipfs
            .pubsub_subscribe(config.id.clone())
            .await?
            .map(|msg| match bincode::deserialize(&msg.data) {
                Ok(kv_msg) => Ok((msg.source, kv_msg)),
                Err(e) => Err(anyhow!(e)),
            });
        config.request_heads().await?;
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

    pub fn serialize<S>(vec: &[Cid], ser: S) -> Result<S::Ok, S::Error>
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
    Block::encode(DagCborCodec, Code::Blake3_256, data)
}

fn to_block_raw<T: AsRef<[u8]>>(data: &T) -> Result<Block> {
    Block::encode(RawCodec, Code::Blake3_256, data.as_ref())
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
                    if let Err(e) = store.try_merge_heads(heads.into_iter()).await {
                        error!("failed to merge heads {}", e);
                    };
                }
                Ok((p, KVMessage::StateReq)) => {
                    debug!("{} requests state", p);
                    // send heads
                    if let Err(e) = store.broadcast_heads().await {
                        error!("failed to broadcast heads {}", e);
                    };
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
    use ipfs::{Keypair, MultiaddrWithoutPeerId, Protocol};

    use super::*;
    use crate::{ipfs::create_ipfs, relay::RelayNode, tracing_try_init};
    use std::{collections::BTreeMap, convert::TryFrom, path::PathBuf, time::Duration};

    async fn create_store<I>(
        id: &str,
        path: PathBuf,
        keypair: Keypair,
        allowed_peers: I,
    ) -> Result<Store, anyhow::Error>
    where
        I: IntoIterator<Item = PeerId> + 'static,
    {
        std::fs::create_dir(path.clone())?;
        let (ipfs, ipfs_task, _receiver) =
            create_ipfs(id.to_string(), &path, keypair, allowed_peers).await?;
        let db = sled::open(path.join("db.sled"))?;
        tokio::spawn(ipfs_task);
        Store::new(id.to_string(), ipfs, db)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test() -> Result<(), anyhow::Error> {
        tracing_try_init();
        let relay_keypair = Keypair::generate_ed25519();
        let relay_peer_id = relay_keypair.public().into_peer_id();
        let relay = RelayNode::new(10001, relay_keypair)?;

        let tmp = tempdir::TempDir::new("test_streams")?;
        let id = "test_id".to_string();

        let alice_keypair = Keypair::generate_ed25519();
        let alice_peer_id = alice_keypair.public().into_peer_id();
        let bob_keypair = Keypair::generate_ed25519();
        let bob_peer_id = bob_keypair.public().into_peer_id();

        let alice =
            create_store(&id, tmp.path().join("alice"), alice_keypair, [bob_peer_id]).await?;
        let bob = create_store(&id, tmp.path().join("bob"), bob_keypair, [alice_peer_id]).await?;

        println!("1");
        let alice_service = alice.start_service().await?;
        println!("2");
        let bob_service = bob.start_service().await?;
        println!("3");
        std::thread::sleep(Duration::from_millis(500));

        let json = r#"{"hello":"there"}"#;
        let key1 = "my_json.json";
        let key2 = "my_dup_json.json";
        let md: BTreeMap<String, String> =
            [("content-type".to_string(), "application/json".to_string())]
                .to_vec()
                .into_iter()
                .collect();

        println!("4");
        let s3_obj_1 = ObjectBuilder::new(key1.as_bytes().to_vec(), md.clone());
        println!("5");
        let s3_obj_2 = ObjectBuilder::new(key2.as_bytes().to_vec(), md.clone());

        println!("6");
        type RmItem = (Vec<u8>, Option<(u64, Cid)>);
        let rm: Vec<RmItem> = vec![];
        println!("7");
        alice_service
            .write(vec![(s3_obj_1, json.as_bytes())], rm.clone())
            .await?;
        println!("8");
        bob_service
            .write(vec![(s3_obj_2, json.as_bytes())], rm)
            .await?;

        println!("9");
        {
            // ensure only alice has s3_obj_1
            let o = alice_service
                .get(key1)
                .await?
                .expect("object 1 not found for alice");
            assert_eq!(&o.key, key1.as_bytes());
            assert_eq!(&o.metadata, &md);
            assert_eq!(bob_service.get(key1).await?, None);
        };
        println!("10");
        {
            // ensure only bob has s3_obj_2
            let o = bob_service
                .get(key2)
                .await?
                .expect("object 2 not found for bob");
            assert_eq!(&o.key, key2.as_bytes());
            assert_eq!(&o.metadata, &md);
            assert_eq!(alice_service.get(key2).await?, None);
        };
        println!("11");

        bob_service
            .ipfs
            .connect(
                MultiaddrWithoutPeerId::try_from(
                    relay
                        .external()
                        .with(Protocol::P2p(relay_peer_id.into()))
                        .with(Protocol::P2pCircuit),
                )?
                .with(alice_peer_id.clone()),
            )
            .await?;

        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(
            bob_service
                .get(key1)
                .await?
                .expect("object 1 not found for bob"),
            alice_service
                .get(key1)
                .await?
                .expect("object 1 not found for alice")
        );
        println!("12");
        assert_eq!(
            bob_service
                .get(key2)
                .await?
                .expect("object 2 not found for bob"),
            alice_service
                .get(key2)
                .await?
                .expect("object 2 not found for alice")
        );
        println!("13");

        // remove key1
        let add: Vec<(&[u8], Cid)> = vec![];
        alice_service
            .index(add, vec![(key1.as_bytes().to_vec(), None)])
            .await?;

        assert_eq!(alice_service.get(key1).await?, None);

        std::thread::sleep(Duration::from_millis(500));

        assert_eq!(bob_service.get(key1).await?, None);

        Ok(())
    }
}
