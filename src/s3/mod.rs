use anyhow::Result;
use ipfs::PeerId;
use libipld::{cbor::DagCborCodec, cid::Cid, codec::Encode, multihash::Code, raw::RawCodec};
use rocket::futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub mod behaviour;
mod entries;
mod store;

use super::{
    ipfs::{Block, Ipfs},
    orbit::AbortOnDrop,
};
use crate::heads::HeadStore;

pub use entries::{Object, ObjectBuilder, ObjectReader};
pub use store::Store;

type TaskHandle = AbortOnDrop<()>;

#[derive(Clone)]
pub struct Service<H> {
    pub store: Store,
    _task: Arc<TaskHandle>,
}

impl<H: HeadStore> Service<H> {
    pub(crate) fn new(store: Store<H>, task: TaskHandle) -> Self {
        Self {
            store,
            _task: Arc::new(task),
        }
    }
}

impl<H> Service<H>
where
    H: 'static + HeadStore + Send + Sync + Clone,
{
    pub async fn start(store: Store<H>) -> Result<Self> {
        let events = store
            .ipfs
            .pubsub_subscribe(store.id.clone())
            .await?
            .map(|msg| match bincode::deserialize(&msg.data) {
                Ok(kv_msg) => Ok((msg.source, kv_msg)),
                Err(e) => Err(anyhow!(e)),
            });
        let peer_id = store.ipfs.identity().await?.0.to_peer_id();
        let task = AbortOnDrop::new(tokio::spawn(kv_task(events, store.clone(), peer_id)));
        store.request_heads().await?;
        Ok(Service::new(store, task))
    }
}

impl<H> std::ops::Deref for Service<H> {
    type Target = Store<H>;
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

async fn kv_task<H>(
    events: impl Stream<Item = Result<(PeerId, KVMessage)>> + Send,
    store: Store<H>,
    peer_id: PeerId,
) where
    H: 'static + HeadStore + Send + Sync + Clone,
{
    debug!("starting KV task");
    events
        .for_each_concurrent(None, |ev| async {
            match ev {
                Ok((p, ev)) if p == peer_id => {
                    debug!("{} filtered out this event from self: {:?}", p, ev)
                }
                Ok((p, KVMessage::Heads(heads))) => {
                    debug!("{} received new heads from {}", peer_id, p);
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
    use crate::{
        config, heads::SledHeadStore, ipfs::create_ipfs, relay::test::test_relay, tracing_try_init,
    };
    use std::{
        collections::BTreeMap, convert::TryFrom, path::PathBuf, str::FromStr, time::Duration,
    };

    async fn create_store<I>(
        id: &Cid,
        path: PathBuf,
        keypair: Keypair,
        allowed_peers: I,
    ) -> Result<(Store<SledHeadStore>, behaviour::BehaviourProcess), anyhow::Error>
    where
        I: IntoIterator<Item = PeerId> + 'static,
    {
        std::fs::create_dir(path.clone())?;
        let config = config::Config {
            storage: config::Storage {
                blocks: config::BlockStorage::Local(config::LocalBlockStorage {
                    path: path.clone(),
                }),
                indexes: config::IndexStorage::Local(config::LocalIndexStorage {
                    path: path.clone(),
                }),
            },
            ..Default::default()
        };
        let (ipfs, ipfs_task, receiver) = create_ipfs(*id, &config, keypair, allowed_peers).await?;
        let db = sled::open(path.join("db.sled"))?;
        let heads = SledHeadStore::new(db)?;
        tokio::spawn(ipfs_task);
        let store = Store::new(id.to_string(), ipfs, db, heads)?;
        Ok((
            store.clone(),
            behaviour::BehaviourProcess::new(store, receiver),
        ))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test() -> Result<(), anyhow::Error> {
        tracing_try_init();
        let relay = test_relay().await?;
        let relay_peer_id = relay.id.clone();
        let relay_internal = relay.internal();

        let tmp = tempdir::TempDir::new("test_streams")?;
        let id =
            Cid::from_str("bafkreieq5jui4j25lacwomsqgjeswwl3y5zcdrresptwgmfylxo2depppq").unwrap();

        let alice_keypair = Keypair::generate_ed25519();
        let alice_peer_id = alice_keypair.public().to_peer_id();
        let bob_keypair = Keypair::generate_ed25519();
        let bob_peer_id = bob_keypair.public().to_peer_id();

        let (alice_store, _alice_behaviour_process) =
            create_store(&id, tmp.path().join("alice"), alice_keypair, [bob_peer_id]).await?;
        let (bob_store, _bob_behaviour_process) =
            create_store(&id, tmp.path().join("bob"), bob_keypair, [bob_peer_id]).await?;

        let alice_service = alice_store.start_service().await?;
        let bob_service = bob_store.start_service().await?;

        // Connect the peers to the relay.
        alice_service
            .ipfs
            .connect(
                MultiaddrWithoutPeerId::try_from(relay_internal.clone())?
                    .with(relay_peer_id.clone()),
            )
            .await
            .expect("alice failed to connect to relay");
        bob_service
            .ipfs
            .connect(
                MultiaddrWithoutPeerId::try_from(relay_internal.clone())?
                    .with(relay_peer_id.clone()),
            )
            .await
            .expect("bob failed to connect to relay");

        // Connect the peers to eachother.
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
            .await
            .expect("bob failed to connect to alice");

        // TODO: Work out why there is a race condition, and fix it so we don't need this sleep between connecting and writing.
        tokio::time::sleep(Duration::from_millis(50)).await;

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

        type RmItem = (Vec<u8>, Option<(u64, Cid)>);
        let rm: Vec<RmItem> = vec![];
        alice_service
            .write(vec![(s3_obj_1, json.as_bytes())], rm.clone())
            .await?;
        bob_service
            .write(vec![(s3_obj_2, json.as_bytes())], rm)
            .await?;

        {
            // ensure only alice has s3_obj_1
            let o = alice_service
                .get(key1)
                .await?
                .expect("object 1 not found for alice");
            assert_eq!(&o.key, key1.as_bytes());
            assert_eq!(&o.metadata, &md);
            assert_eq!(bob_service.get(key1).await?, None, "object 1 found for bob");
        };
        {
            // ensure only bob has s3_obj_2
            let o = bob_service
                .get(key2)
                .await?
                .expect("object 2 not found for bob");
            assert_eq!(&o.key, key2.as_bytes());
            assert_eq!(&o.metadata, &md);
            assert_eq!(
                alice_service.get(key2).await?,
                None,
                "object 2 found for alice"
            );
        };

        tokio::time::sleep(Duration::from_millis(500)).await;

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

        // remove key1
        let add: Vec<(&[u8], Cid)> = vec![];
        alice_service
            .index(add, vec![(key1.as_bytes().to_vec(), None)])
            .await?;

        assert_eq!(
            alice_service.get(key1).await?,
            None,
            "alice still has object 1"
        );

        std::thread::sleep(Duration::from_millis(500));

        assert_eq!(bob_service.get(key1).await?, None, "bob still has object 1");

        Ok(())
    }
}
