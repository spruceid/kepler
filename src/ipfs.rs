use anyhow::Result;
use kepler_lib::libipld::{
    block::Block as OBlock,
    cid::{multibase::Base, Cid},
    multihash::Code,
    raw::RawCodec,
    store::DefaultParams,
};
use libp2p::{core::transport::MemoryTransport, futures::TryStreamExt, swarm::Swarm as TSwarm};
use std::{future::Future, sync::mpsc::Receiver};

use super::{cas::ContentAddressedStorage, codec::SupportedCodecs};
use crate::{
    config,
    kv::behaviour::{Behaviour, Event as BehaviourEvent},
    storage::{Repo, StorageUtils},
};

pub type KeplerParams = DefaultParams;
// #[derive(Clone, Debug, Default)]
// pub struct KeplerParams;

// impl StoreParams for KeplerParams {
//     const MAX_BLOCK_SIZE: usize = 10_485_760;
//     type Codecs = IpldCodec;
//     type Hashes = Code;
// }

pub type Block = OBlock<KeplerParams>;
pub type Swarm = TSwarm<Types>;

pub async fn create_ipfs<I>(
    orbit: Cid,
    config: &config::Config,
    keypair: Keypair,
    allowed_peers: I,
) -> Result<(Ipfs, impl Future<Output = ()>, Receiver<BehaviourEvent>)>
where
    I: IntoIterator<Item = PeerId> + 'static,
{
    let storage_utils = StorageUtils::new(config.storage.blocks.clone());

    let ipfs_opts = IpfsOptions {
        ipfs_path: storage_utils.ipfs_path(orbit).await?,
        keypair,
        bootstrap: vec![],
        mdns: false,
        kad_protocol: Some(format!(
            "/kepler/{}",
            orbit.to_string_of_base(Base::Base58Btc)?
        )),
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
        UninitializedIpfs::<Repo>::new(ipfs_opts, transport, Some(relay_behaviour))
            .with_extended_behaviour(behaviour)
            .start()
            .await?;

    Ok((ipfs, ipfs_task, receiver))
}
