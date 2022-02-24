use std::{future::Future, path::Path, sync::mpsc::Receiver};

use crate::s3::behaviour::{Behaviour, Event as BehaviourEvent};

use super::cas::ContentAddressedStorage;
use super::codec::SupportedCodecs;
use anyhow::Result;
use ipfs::{
    multiaddr,
    p2p::{transport::TransportBuilder, TSwarm},
    path::PathRoot,
    Ipfs as OIpfs, IpfsOptions, IpfsPath, Keypair, PeerId, PinMode, Types, UninitializedIpfs,
};
use libipld::{
    block::Block as OBlock,
    cid::Cid,
    store::{DefaultParams, StoreParams},
    Ipld,
};
use libp2p::{core::transport::MemoryTransport, futures::TryStreamExt};

pub type KeplerParams = DefaultParams;
// #[derive(Clone, Debug, Default)]
// pub struct KeplerParams;

// impl StoreParams for KeplerParams {
//     const MAX_BLOCK_SIZE: usize = 10_485_760;
//     type Codecs = IpldCodec;
//     type Hashes = Code;
// }

pub type Ipfs = OIpfs<Types>;
pub type Block = OBlock<KeplerParams>;
pub type Swarm = TSwarm<Types>;

pub async fn create_ipfs<'l, I>(
    id: String,
    dir: &'l Path,
    keypair: Keypair,
    allowed_peers: I,
) -> Result<(Ipfs, impl Future<Output = ()>, Receiver<BehaviourEvent>)>
where
    I: IntoIterator<Item = PeerId> + 'static,
{
    let ipfs_path = dir.join("ipfs");
    std::fs::create_dir(&ipfs_path)?;
    let ipfs_opts = IpfsOptions {
        ipfs_path,
        keypair,
        bootstrap: vec![],
        mdns: false,
        kad_protocol: None,
        listening_addrs: vec![multiaddr!(P2pCircuit)],
        span: None,
    };

    let (sender, receiver) = std::sync::mpsc::sync_channel::<BehaviourEvent>(100);
    let behaviour = Behaviour::new(sender);

    let (transport_builder, relay_behaviour) = TransportBuilder::new(ipfs_opts.keypair.clone())?
        .or(MemoryTransport::default())
        .relay();

    let transport = transport_builder
        .map_auth()
        .map(crate::transport::auth_mapper(allowed_peers))
        .build();

    let (ipfs, ipfs_task) =
        UninitializedIpfs::<Types>::new(ipfs_opts, transport, Some(relay_behaviour))
            .with_extended_behaviour(behaviour)
            .start()
            .await?;

    Ok((ipfs, ipfs_task, receiver))
}

pub async fn relay(port: u16) -> OIpfs<ipfs::TestTypes> {
    let mut ipfs_opts = IpfsOptions::inmemory_with_generated_keys();
    ipfs_opts.listening_addrs = vec![multiaddr!(Memory(port))];

    let (transport_builder, relay_behaviour) = TransportBuilder::new(ipfs_opts.keypair.clone())
        .unwrap()
        .or(MemoryTransport::default())
        .relay();
    let (ipfs, ipfs_task) =
        UninitializedIpfs::new(ipfs_opts, transport_builder.build(), Some(relay_behaviour))
            .start()
            .await
            .unwrap();
    tokio::task::spawn(ipfs_task);
    return ipfs;
}

#[rocket::async_trait]
impl ContentAddressedStorage for Ipfs {
    type Error = anyhow::Error;
    async fn put(&self, content: &[u8], _codec: SupportedCodecs) -> Result<Cid, Self::Error> {
        let parts: Vec<Ipld> = content
            .chunks(KeplerParams::MAX_BLOCK_SIZE)
            .map(Vec::from)
            .map(Ipld::Bytes)
            .collect();

        let dag = Ipld::List(parts);
        let cid = self.put_dag(dag).await?;
        self.insert_pin(&cid, true).await?;
        Ok(cid)
    }
    async fn get(&self, address: &Cid) -> Result<Option<Vec<u8>>, Self::Error> {
        // TODO this api returns Result<Block, anyhow::Error>, with an err thrown for no block found
        // until this API changes (a breaking change), we will error here when no block found
        if let Ipld::List(parts) = self
            .get_dag(IpfsPath::new(PathRoot::Ipld(address.clone())))
            .await?
        {
            return Ok(Some(parts.into_iter().try_fold(
                vec![],
                |mut acc, ipld| {
                    if let Ipld::Bytes(mut part) = ipld {
                        acc.append(&mut part);
                        return Ok(acc);
                    }
                    Err(anyhow!("unexpected structure"))
                },
            )?));
        }
        Err(anyhow!("unexpected structure"))
    }

    async fn delete(&self, address: &Cid) -> Result<(), Self::Error> {
        // TODO: does not recursively remove blocks, some cleanup will need to happen.
        self.remove_pin(address, true).await?;
        self.remove_block(address.clone()).await?;
        Ok(())
    }
    async fn list(&self) -> Result<Vec<Cid>, Self::Error> {
        // return a list of all CIDs which are aliased/pinned
        self.list_pins(Some(PinMode::Recursive))
            .await
            .map_ok(|(cid, _pin_mode)| cid)
            .try_collect()
            .await
    }
}
