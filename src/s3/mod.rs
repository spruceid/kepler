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
    let this_pid = store.ipfs.identity().await.unwrap().0.into_peer_id();
    events
        .for_each_concurrent(None, |ev| async {
            match ev {
                Ok((p, ev)) if p == this_pid => {
                    info!("{} filtered out this event from self: {:?}", p, ev)
                },
                Ok((p, KVMessage::Heads(heads))) => {
                    info!(
                        "{} new heads from {}",
                        store.ipfs.identity().await.unwrap().0.into_peer_id(),
                        p
                    );
                    // sync heads
                    if let Err(e) = store.try_merge_heads(heads.into_iter()).await {
                        error!("failed to merge heads {}", e);
                    };
                }
                Ok((p, KVMessage::StateReq)) => {
                    info!("{} requests state", p);
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
    use ipfs::{Keypair, MultiaddrWithoutPeerId, Protocol, multiaddr};

    use super::*;
    use crate::{ipfs::create_ipfs, relay::RelayNode, tracing_try_init};
    use std::{collections::BTreeMap, convert::TryFrom, path::PathBuf, time::Duration};

    async fn create_store<I>(
        id: &str,
        path: PathBuf,
        keypair: Keypair,
        allowed_peers: I,
    ) -> Result<(Store, behaviour::BehaviourProcess), anyhow::Error>
    where
        I: IntoIterator<Item = PeerId> + 'static,
    {
        std::fs::create_dir(path.clone())?;
        let (ipfs, ipfs_task, receiver) =
            create_ipfs(id.to_string(), &path, keypair, allowed_peers).await?;
        let db = sled::open(path.join("db.sled"))?;
        tokio::spawn(ipfs_task);
        let store = Store::new(id.to_string(), ipfs, db)?;
        Ok((
            store.clone(),
            behaviour::BehaviourProcess::new(store, receiver),
        ))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test() -> Result<(), anyhow::Error> {
        tracing_try_init();
        let relay = crate::ipfs::relay(10001).await;
        let relay_peer_id = relay.identity().await?.0.into_peer_id();
        let relay_internal = multiaddr!(Memory(10001u16));

        let tmp = tempdir::TempDir::new("test_streams")?;
        let id = "test_id".to_string();

        let alice_keypair = Keypair::generate_ed25519();
        let alice_peer_id = alice_keypair.public().into_peer_id();
        let bob_keypair = Keypair::generate_ed25519();
        let bob_peer_id = bob_keypair.public().into_peer_id();

        let (alice_store, _alice_behaviour_process) =
            create_store(&id, tmp.path().join("alice"), alice_keypair, [bob_peer_id]).await?;
        let (bob_store, _bob_behaviour_process) =
            create_store(&id, tmp.path().join("bob"), bob_keypair, [bob_peer_id]).await?;

        let alice_service = alice_store.start_service().await?;
        let bob_service = bob_store.start_service().await?;
        //alice_service
        //    .ipfs
        //    .connect(
        //        MultiaddrWithoutPeerId::try_from(relay_internal.clone())?.with(relay_peer_id.clone()),
        //    )
        //    .await
        //    .expect("alice failed to connect to relay");
        //bob_service
        //    .ipfs
        //    .connect(
        //        MultiaddrWithoutPeerId::try_from(relay_internal.clone())?.with(relay_peer_id.clone()),
        //    )
        //    .await
        //    .expect("bob failed to connect to relay");

        bob_service
            .ipfs
            .connect(
                MultiaddrWithoutPeerId::try_from(
                    multiaddr!(Memory(10002u16))
                    //relay_internal
                    //    .with(Protocol::P2p(relay_peer_id.into()))
                    //    .with(Protocol::P2pCircuit),
                )?
                .with(alice_peer_id.clone()),
            )
            .await
            .expect("bob failed to connect to alice");

        let peers = bob_service.ipfs.peers().await?;
        tracing::info!(
            "{:?}",
            peers.iter().map(|c| c.addr.peer_id).collect::<Vec<_>>()
        );

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
        let peers = bob_service.ipfs.peers().await?;
        tracing::info!(
            "{:#?}",
            peers
        );
        tracing::info!("11");

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
        tracing::info!("12");
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
        tracing::info!("13");

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
