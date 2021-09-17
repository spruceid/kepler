pub mod db;
pub mod executor;
pub mod net;
#[cfg(feature = "telemetry")]
mod telemetry;
#[cfg(test)]
mod test_util;

pub use db::open_store;
use libipld::{store::DefaultParams, Block, Cid, Result};
use libp2p::{multiaddr::Protocol, Multiaddr};
use net::BitswapStore;
pub use net::{open_relay, NetworkService};
use std::path::PathBuf;
#[cfg(feature = "telemetry")]
pub use telemetry::telemetry;

pub async fn open_orbit_ipfs(
    oid: Cid,
    dir: PathBuf,
    relay_addr: Multiaddr,
) -> Result<NetworkService<DefaultParams>> {
    let executor = executor::Executor::new();
    let bitswap = BitswapStorage {
        oid,
        dir: dir.clone(),
    };
    let network_service = NetworkService::new(bitswap, executor, dir).await?;
    let addr = relay_addr.with(Protocol::P2p(network_service.local_peer_id().into()));
    let _ = network_service.listen_on(addr)?;
    Ok(network_service)
}

struct BitswapStorage {
    oid: Cid,
    dir: PathBuf,
}

impl BitswapStore for BitswapStorage {
    type Params = DefaultParams;

    fn contains(&mut self, cid: &Cid) -> Result<bool> {
        let store = open_store(self.oid, self.dir.clone())?;
        store.contains(cid)
    }

    fn get(&mut self, cid: &Cid) -> Result<Option<Vec<u8>>> {
        let store = open_store(self.oid, self.dir.clone())?;
        store.get(cid)
    }

    fn insert(&mut self, block: &Block<Self::Params>) -> Result<()> {
        let store = open_store(self.oid, self.dir.clone())?;
        store.insert(block)
    }

    fn missing_blocks(&mut self, cid: &Cid) -> Result<Vec<Cid>> {
        let store = open_store(self.oid, self.dir.clone())?;
        store.missing_blocks(cid)
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use async_std::future::timeout;
//     use futures::join;
//     use futures::stream::StreamExt;
//     use libipld::cbor::DagCborCodec;
//     use libipld::multihash::Code;
//     use libipld::raw::RawCodec;
//     use libipld::store::DefaultParams;
//     use libipld::{alias, ipld};
//     use std::time::Duration;
//     use tempdir::TempDir;

//     fn tracing_try_init() {
//         tracing_subscriber::fmt()
//             .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
//             .try_init()
//             .ok();
//     }

//     async fn create_store() -> Result<(Ipfs<DefaultParams>, TempDir)> {
//         let tmp = TempDir::new("ipfs-embed")?;
//         let sweep_interval = Duration::from_millis(10000);
//         let storage = StorageConfig::new(None, 10, sweep_interval);

//         let network = NetworkConfig::new(tmp.path().into(), generate_keypair());

//         let ipfs = Ipfs::new(Config { storage, network }).await?;
//         ipfs.listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())?
//             .next()
//             .await
//             .unwrap();
//         Ok((ipfs, tmp))
//     }

//     fn create_block(bytes: &[u8]) -> Result<Block<DefaultParams>> {
//         Block::encode(RawCodec, Code::Blake3_256, bytes)
//     }

//     #[async_std::test]
//     async fn test_local_store() -> Result<()> {
//         tracing_try_init();
//         let (store, _tmp) = create_store().await?;
//         let block = create_block(b"test_local_store")?;
//         let tmp = store.create_temp_pin()?;
//         store.temp_pin(&tmp, block.cid())?;
//         let _ = store.insert(&block)?;
//         let block2 = store.get(block.cid())?;
//         assert_eq!(block.data(), block2.data());
//         Ok(())
//     }

//     #[async_std::test]
//     #[ignore] // test is too unreliable for ci
//     async fn test_exchange_kad() -> Result<()> {
//         tracing_try_init();
//         let (store, _tmp) = create_store().await?;
//         let (store1, _tmp) = create_store().await?;
//         let (store2, _tmp) = create_store().await?;

//         let addr = store.listeners()[0].clone();
//         let peer_id = store.local_peer_id();
//         let nodes = [(peer_id, addr)];

//         let b1 = store1.bootstrap(&nodes);
//         let b2 = store2.bootstrap(&nodes);
//         let (r1, r2) = join!(b1, b2);
//         r1.unwrap();
//         r2.unwrap();

//         let block = create_block(b"test_exchange_kad")?;
//         let key = Key::new(&block.cid().to_bytes());
//         let tmp1 = store1.create_temp_pin()?;
//         store1.temp_pin(&tmp1, block.cid())?;
//         store1.insert(&block)?;
//         store1.provide(key.clone()).await?;
//         store1.flush().await?;

//         let tmp2 = store2.create_temp_pin()?;
//         store2.temp_pin(&tmp2, block.cid())?;
//         let providers = store2.providers(key).await?;
//         let block2 = store2
//             .fetch(block.cid(), providers.into_iter().collect())
//             .await?;
//         assert_eq!(block.data(), block2.data());
//         Ok(())
//     }

//     // #[async_std::test]
//     // async fn test_provider_not_found() -> Result<()> {
//     //     tracing_try_init();
//     //     let (store1, _tmp) = create_store(true).await?;
//     //     let block = create_block(b"test_provider_not_found")?;
//     //     if store1
//     //         .fetch(block.cid(), vec![store1.local_peer_id()])
//     //         .await
//     //         .unwrap_err()
//     //         .downcast_ref::<BlockNotFound>()
//     //         .is_none()
//     //     {
//     //         panic!("expected block not found error");
//     //     }
//     //     Ok(())
//     // }

//     macro_rules! assert_pinned {
//         ($store:expr, $block:expr) => {
//             assert_eq!(
//                 $store
//                     .reverse_alias($block.cid())
//                     .unwrap()
//                     .map(|a| !a.is_empty()),
//                 Some(true)
//             );
//         };
//     }

//     macro_rules! assert_unpinned {
//         ($store:expr, $block:expr) => {
//             assert_eq!(
//                 $store
//                     .reverse_alias($block.cid())
//                     .unwrap()
//                     .map(|a| !a.is_empty()),
//                 Some(false)
//             );
//         };
//     }

//     fn create_ipld_block(ipld: &Ipld) -> Result<Block<DefaultParams>> {
//         Block::encode(DagCborCodec, Code::Blake3_256, ipld)
//     }

//     #[async_std::test]
//     async fn test_sync() -> Result<()> {
//         tracing_try_init();
//         let (local1, _tmp) = create_store().await?;
//         let (local2, _tmp) = create_store().await?;
//         local1.add_address(&local2.local_peer_id(), local2.listeners()[0].clone());
//         local2.add_address(&local1.local_peer_id(), local1.listeners()[0].clone());

//         let a1 = create_ipld_block(&ipld!({ "a": 0 }))?;
//         let b1 = create_ipld_block(&ipld!({ "b": 0 }))?;
//         let c1 = create_ipld_block(&ipld!({ "c": [a1.cid(), b1.cid()] }))?;
//         let b2 = create_ipld_block(&ipld!({ "b": 1 }))?;
//         let c2 = create_ipld_block(&ipld!({ "c": [a1.cid(), b2.cid()] }))?;
//         let x = alias!(x);

//         let _ = local1.insert(&a1)?;
//         let _ = local1.insert(&b1)?;
//         let _ = local1.insert(&c1)?;
//         local1.alias(x, Some(c1.cid()))?;
//         local1.flush().await?;
//         assert_pinned!(&local1, &a1);
//         assert_pinned!(&local1, &b1);
//         assert_pinned!(&local1, &c1);

//         local2.alias(&x, Some(c1.cid()))?;
//         local2.sync(c1.cid(), vec![local1.local_peer_id()]).await?;
//         local2.flush().await?;
//         assert_pinned!(&local2, &a1);
//         assert_pinned!(&local2, &b1);
//         assert_pinned!(&local2, &c1);

//         let _ = local2.insert(&b2)?;
//         let _ = local2.insert(&c2)?;
//         local2.alias(x, Some(c2.cid()))?;
//         local2.flush().await?;
//         assert_pinned!(&local2, &a1);
//         assert_unpinned!(&local2, &b1);
//         assert_unpinned!(&local2, &c1);
//         assert_pinned!(&local2, &b2);
//         assert_pinned!(&local2, &c2);

//         local1.alias(x, Some(c2.cid()))?;
//         local1.sync(c2.cid(), vec![local2.local_peer_id()]).await?;
//         local1.flush().await?;
//         assert_pinned!(&local1, &a1);
//         assert_unpinned!(&local1, &b1);
//         assert_unpinned!(&local1, &c1);
//         assert_pinned!(&local1, &b2);
//         assert_pinned!(&local1, &c2);

//         local2.alias(x, None)?;
//         local2.flush().await?;
//         assert_unpinned!(&local2, &a1);
//         assert_unpinned!(&local2, &b1);
//         assert_unpinned!(&local2, &c1);
//         assert_unpinned!(&local2, &b2);
//         assert_unpinned!(&local2, &c2);

//         local1.alias(x, None)?;
//         local2.flush().await?;
//         assert_unpinned!(&local1, &a1);
//         assert_unpinned!(&local1, &b1);
//         assert_unpinned!(&local1, &c1);
//         assert_unpinned!(&local1, &b2);
//         assert_unpinned!(&local1, &c2);
//         Ok(())
//     }

//     #[async_std::test]
//     #[allow(clippy::eval_order_dependence)]
//     async fn test_dht_record() -> Result<()> {
//         tracing_try_init();
//         let stores = [create_store().await?, create_store().await?];
//         async_std::task::sleep(Duration::from_millis(100)).await;
//         stores[0]
//             .0
//             .bootstrap(&[(
//                 stores[1].0.local_peer_id(),
//                 stores[1].0.listeners()[0].clone(),
//             )])
//             .await?;
//         stores[1]
//             .0
//             .bootstrap(&[(
//                 stores[0].0.local_peer_id(),
//                 stores[0].0.listeners()[0].clone(),
//             )])
//             .await?;
//         let key: Key = b"key".to_vec().into();

//         stores[0]
//             .0
//             .put_record(
//                 Record::new(key.clone(), b"hello world".to_vec()),
//                 Quorum::One,
//             )
//             .await?;
//         let records = stores[1].0.get_record(&key, Quorum::One).await?;
//         assert_eq!(records.len(), 1);
//         Ok(())
//     }

//     #[async_std::test]
//     #[allow(clippy::eval_order_dependence)]
//     async fn test_gossip_and_broadcast() -> Result<()> {
//         tracing_try_init();
//         let stores = [
//             create_store().await?,
//             create_store().await?,
//             create_store().await?,
//             create_store().await?,
//             create_store().await?,
//             create_store().await?,
//         ];
//         let mut subscriptions = vec![];
//         let topic = "topic";
//         for (store, _) in &stores {
//             for (other, _) in &stores {
//                 if store.local_peer_id() != other.local_peer_id() {
//                     store.dial_address(&other.local_peer_id(), other.listeners()[0].clone());
//                 }
//             }
//         }

//         async_std::task::sleep(Duration::from_millis(500)).await;
//         // Make sure everyone is peered before subscribing
//         for (store, _) in &stores {
//             subscriptions.push(store.subscribe(topic)?);
//         }
//         async_std::task::sleep(Duration::from_millis(500)).await;

//         stores[0]
//             .0
//             .publish(topic, b"hello gossip".to_vec())
//             .unwrap();

//         for (idx, subscription) in subscriptions.iter_mut().enumerate() {
//             let mut expected = stores
//                 .iter()
//                 .enumerate()
//                 .filter_map(|(i, s)| {
//                     if i == idx {
//                         None
//                     } else {
//                         Some(s.0.local_peer_id())
//                     }
//                 })
//                 .flat_map(|p| {
//                     // once for gossipsub, once for broadcast
//                     vec![GossipEvent::Subscribed(p), GossipEvent::Subscribed(p)].into_iter()
//                 })
//                 .chain(if idx != 0 {
//                     // store 0 is the sender
//                     Box::new(std::iter::once(GossipEvent::Message(
//                         stores[0].0.local_peer_id(),
//                         b"hello gossip".to_vec().into(),
//                     ))) as Box<dyn Iterator<Item = GossipEvent>>
//                 } else {
//                     Box::new(std::iter::empty())
//                 })
//                 .collect::<Vec<GossipEvent>>();
//             while !expected.is_empty() {
//                 let ev = timeout(Duration::from_millis(100), subscription.next())
//                     .await
//                     .unwrap()
//                     .unwrap();
//                 assert!(expected.contains(&ev));
//                 if let Some(idx) = expected.iter().position(|e| e == &ev) {
//                     // Can't retain, as there might be multiple messages
//                     expected.remove(idx);
//                 }
//             }
//         }

//         // Check broadcast subscription
//         stores[0]
//             .0
//             .broadcast(topic, b"hello broadcast".to_vec())
//             .unwrap();

//         for subscription in &mut subscriptions[1..] {
//             if let GossipEvent::Message(p, data) = subscription.next().await.unwrap() {
//                 assert_eq!(p, stores[0].0.local_peer_id());
//                 assert_eq!(data[..], b"hello broadcast"[..]);
//             } else {
//                 panic!()
//             }
//         }

//         // trigger cleanup
//         stores[0]
//             .0
//             .broadcast(topic, b"r u still listening?".to_vec())
//             .unwrap();

//         let mut last_sub = subscriptions.drain(..1).next().unwrap();
//         drop(subscriptions);
//         let mut expected = stores[1..]
//             .iter()
//             .map(|s| s.0.local_peer_id())
//             .flat_map(|p| {
//                 // once for gossipsub, once for broadcast
//                 vec![GossipEvent::Unsubscribed(p), GossipEvent::Unsubscribed(p)].into_iter()
//             })
//             .collect::<Vec<_>>();
//         while !expected.is_empty() {
//             let ev = timeout(Duration::from_millis(100), last_sub.next())
//                 .await
//                 .unwrap()
//                 .unwrap();
//             assert!(expected.contains(&ev));
//             if let Some(idx) = expected.iter().position(|e| e == &ev) {
//                 // Can't retain, as there might be multiple messages
//                 expected.remove(idx);
//             }
//         }
//         Ok(())
//     }

//     #[async_std::test]
//     async fn test_batch_read() -> Result<()> {
//         tracing_try_init();
//         let tmp = TempDir::new("ipfs-embed")?;
//         let network = NetworkConfig::new(tmp.path().into(), generate_keypair());
//         let storage = StorageConfig::new(None, 1000000, Duration::from_secs(3600));
//         let ipfs = Ipfs::<DefaultParams>::new(Config { storage, network }).await?;
//         let a = create_block(b"a")?;
//         let b = create_block(b"b")?;
//         ipfs.insert(&a)?;
//         ipfs.insert(&b)?;
//         let has_blocks =
//             ipfs.read_batch(|db| Ok(db.contains(a.cid())? && db.contains(b.cid())?))?;
//         assert!(has_blocks);
//         Ok(())
//     }

//     #[async_std::test]
//     async fn test_batch_write() -> Result<()> {
//         tracing_try_init();
//         let tmp = TempDir::new("ipfs-embed")?;
//         let network = NetworkConfig::new(tmp.path().into(), generate_keypair());
//         let storage = StorageConfig::new(None, 1000000, Duration::from_secs(3600));
//         let ipfs = Ipfs::<DefaultParams>::new(Config { storage, network }).await?;
//         let a = create_block(b"a")?;
//         let b = create_block(b"b")?;
//         let c = create_block(b"c")?;
//         let d = create_block(b"d")?;
//         ipfs.write_batch(|db| {
//             db.insert(&a)?;
//             db.insert(&b)?;
//             Ok(())
//         })?;
//         assert!(ipfs.contains(a.cid())? && ipfs.contains(b.cid())?);
//         let _: anyhow::Result<()> = ipfs.write_batch(|db| {
//             db.insert(&c)?;
//             db.insert(&d)?;
//             anyhow::bail!("nope!");
//         });
//         assert!(!ipfs.contains(c.cid())? && ipfs.contains(b.cid())?);
//         Ok(())
//     }

//     // #[async_std::test]
//     // #[ignore]
//     // async fn test_bitswap_sync_chain() -> Result<()> {
//     //     use std::time::Instant;
//     //     tracing_try_init();
//     //     let (a, _tmp) = create_store(true).await?;
//     //     let (b, _tmp) = create_store(true).await?;
//     //     let root = alias!(root);

//     //     let (cid, blocks) = test_util::build_tree(1, 1000)?;
//     //     a.alias(root, Some(&cid))?;
//     //     b.alias(root, Some(&cid))?;

//     //     let size: usize = blocks.iter().map(|block| block.data().len()).sum();
//     //     tracing::info!("chain built {} blocks, {} bytes", blocks.len(), size);
//     //     for block in blocks.iter() {
//     //         let _ = a.insert(block)?;
//     //     }
//     //     a.flush().await?;

//     //     let t0 = Instant::now();
//     //     let _ = b
//     //         .sync(&cid, vec![a.local_peer_id()])
//     //         .for_each(|x| async move { tracing::debug!("sync progress {:?}", x) })
//     //         .await;
//     //     b.flush().await?;
//     //     tracing::info!(
//     //         "chain sync complete {} ms {} blocks {} bytes!",
//     //         t0.elapsed().as_millis(),
//     //         blocks.len(),
//     //         size
//     //     );
//     //     for block in blocks {
//     //         let data = b.get(block.cid())?;
//     //         assert_eq!(data, block);
//     //     }

//     //     Ok(())
//     // }

//     // #[async_std::test]
//     // #[ignore]
//     // async fn test_bitswap_sync_tree() -> Result<()> {
//     //     use std::time::Instant;
//     //     tracing_try_init();
//     //     let (a, _tmp) = create_store(true).await?;
//     //     let (b, _tmp) = create_store(true).await?;
//     //     let root = alias!(root);

//     //     let (cid, blocks) = test_util::build_tree(10, 4)?;
//     //     a.alias(root, Some(&cid))?;
//     //     b.alias(root, Some(&cid))?;

//     //     let size: usize = blocks.iter().map(|block| block.data().len()).sum();
//     //     tracing::info!("chain built {} blocks, {} bytes", blocks.len(), size);
//     //     for block in blocks.iter() {
//     //         let _ = a.insert(block)?;
//     //     }
//     //     a.flush().await?;

//     //     let t0 = Instant::now();
//     //     let _ = b
//     //         .sync(&cid, vec![a.local_peer_id()])
//     //         .for_each(|x| async move { tracing::debug!("sync progress {:?}", x) })
//     //         .await;
//     //     b.flush().await?;
//     //     tracing::info!(
//     //         "tree sync complete {} ms {} blocks {} bytes!",
//     //         t0.elapsed().as_millis(),
//     //         blocks.len(),
//     //         size
//     //     );
//     //     for block in blocks {
//     //         let data = b.get(block.cid())?;
//     //         assert_eq!(data, block);
//     //     }
//     //     Ok(())
//     // }
// }
