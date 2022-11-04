use crate::p2p::IdentifyConfig;
use anyhow::Result;
use derive_builder::Builder;
use libp2p::{
    autonat::{Behaviour as AutoNat, Config as AutoNatConfig},
    core::{identity::Keypair, multiaddr::multiaddr, Multiaddr, PeerId},
    identify::Behaviour as Identify,
    identity::PublicKey,
    ping::{Behaviour as Ping, Config as PingConfig},
    relay::v2::relay::{Config as RelayConfig, Relay},
    swarm::Swarm,
    NetworkBehaviour,
};

pub type RelaySwarm = Swarm<Behaviour>;

pub struct RelayNode {
    pub port: u16,
    pub id: PeerId,
}

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    identify: Identify,
    ping: Ping,
    relay: Relay,
    autonat: AutoNat,
}

#[derive(Builder, Debug)]
#[builder(
    build_fn(skip),
    setter(into),
    name = "BehaviourBuilder",
    derive(Debug),
    pattern = "owned"
)]
pub struct BehaviourConfig {
    #[builder(field(type = "IdentifyConfig"), setter(name = "identify"))]
    _identify: IdentifyConfig,
    #[builder(field(type = "PingConfig"), setter(name = "ping"))]
    _ping: PingConfig,
    #[builder(field(type = "RelayConfig"), setter(name = "relay"))]
    _relay: RelayConfig,
    #[builder(field(type = "AutoNatConfig"), setter(name = "autonat"))]
    _autonat: AutoNatConfig,
}

impl BehaviourBuilder {
    pub fn build(self, pubkey: PublicKey) -> Behaviour {
        let peer_id = pubkey.to_peer_id();
        Behaviour {
            identify: Identify::new(self._identify.to_config(pubkey)),
            ping: Ping::new(self._ping),
            relay: Relay::new(peer_id, self._relay),
            autonat: AutoNat::new(peer_id, self._autonat),
        }
    }
}

impl RelayNode {
    pub async fn new(port: u16, keypair: Keypair) -> Result<Self> {
        let local_public_key = keypair.public();
        let id = local_public_key.to_peer_id();
        // let relay_tcp_addr = Self::_external(port);
        // let relay_mem_addr = Self::_internal(port);

        // let (transport_builder, relay_behaviour) = TransportBuilder::new(keypair.clone())?
        //     .or(MemoryTransport::default())
        //     .relay();

        // let ipfs_opts = IpfsOptions {
        //     ipfs_path: std::env::temp_dir(),
        //     keypair,
        //     bootstrap: vec![],
        //     mdns: false,
        //     kad_protocol: "/kepler/relay".to_string().into(),
        //     listening_addrs: vec![relay_tcp_addr, relay_mem_addr],
        //     span: None,
        // };

        // // TestTypes designates an in-memory Ipfs instance, but this peer won't store data anyway.
        // let (_ipfs, ipfs_task) =
        //     UninitializedIpfs::new(ipfs_opts, transport_builder.build(), Some(relay_behaviour))
        //         .start()
        //         .await?;

        // tracing::debug!(
        //     "opened relay: {} at {}, {}",
        //     id,
        //     Self::_internal(port),
        //     Self::_external(port),
        // );

        // let task = spawn(ipfs_task);
        Ok(Self {
            port,
            // _task: AbortOnDrop::new(task),
            id,
        })
    }

    fn _internal(port: u16) -> Multiaddr {
        multiaddr!(Memory(port))
    }

    pub fn internal(&self) -> Multiaddr {
        Self::_internal(self.port)
    }

    fn _external(port: u16) -> Multiaddr {
        multiaddr!(Ip4([127, 0, 0, 1]), Tcp(port))
    }

    pub fn external(&self) -> Multiaddr {
        Self::_external(self.port)
    }
}

#[cfg(test)]
pub mod test {
    // use super::*;
    // use ipfs::{
    //     p2p::transport::TransportBuilder, IpfsOptions, MultiaddrWithoutPeerId, Types,
    //     UninitializedIpfs,
    // };
    // use libp2p::core::multiaddr::{multiaddr, Protocol};
    // use std::{
    //     convert::TryFrom,
    //     sync::atomic::{AtomicU16, Ordering},
    //     time::Duration,
    // };

    // static PORT: AtomicU16 = AtomicU16::new(10000);

    // pub async fn test_relay() -> Result<RelayNode> {
    //     RelayNode::new(
    //         PORT.fetch_add(1, Ordering::SeqCst),
    //         Keypair::generate_ed25519(),
    //     )
    //     .await
    // }

    // #[tokio::test(flavor = "multi_thread")]
    // async fn relay() -> Result<()> {
    //     let relay = test_relay().await?;

    //     let dir = tempfile::TempDir::new("relay")?;
    //     let alice_path = dir.path().join("alice");
    //     std::fs::create_dir(&alice_path)?;
    //     let bob_path = dir.path().join("bob");
    //     std::fs::create_dir(&bob_path)?;

    //     // Isn't actually in-memory, just provides useful defaults.
    //     let mut alice_opts = IpfsOptions::inmemory_with_generated_keys();
    //     alice_opts.ipfs_path = alice_path;
    //     alice_opts.listening_addrs = vec![multiaddr!(P2pCircuit)];
    //     let mut bob_opts = IpfsOptions::inmemory_with_generated_keys();
    //     bob_opts.ipfs_path = bob_path;

    //     let alice_peer_id = alice_opts.keypair.public().to_peer_id();
    //     let bob_peer_id = bob_opts.keypair.public().to_peer_id();

    //     let (alice_builder, alice_relay) = TransportBuilder::new(alice_opts.keypair.clone())?
    //         .or(MemoryTransport::default())
    //         .relay();
    //     let alice_transport = alice_builder
    //         .map_auth()
    //         .map(crate::transport::auth_mapper([bob_peer_id.clone()]))
    //         .build();
    //     let (alice, task) =
    //         UninitializedIpfs::<Types>::new(alice_opts, alice_transport, Some(alice_relay))
    //             .start()
    //             .await?;
    //     tokio::spawn(task);

    //     let (bob_builder, bob_relay) = TransportBuilder::new(bob_opts.keypair.clone())?
    //         .or(MemoryTransport::default())
    //         .relay();
    //     let (bob, task) =
    //         UninitializedIpfs::<Types>::new(bob_opts, bob_builder.build(), Some(bob_relay))
    //             .start()
    //             .await?;
    //     tokio::spawn(task);

    //     alice
    //         .connect(MultiaddrWithoutPeerId::try_from(relay.internal())?.with(relay.id.clone()))
    //         .await
    //         .expect("alice failed to connect to relay");

    //     bob.connect(
    //         MultiaddrWithoutPeerId::try_from(
    //             relay
    //                 .external()
    //                 .with(Protocol::P2p(relay.id.clone().into()))
    //                 .with(Protocol::P2pCircuit),
    //         )?
    //         .with(alice_peer_id.clone()),
    //     )
    //     .await
    //     .expect("bob failed to connect to alice");

    //     tokio::time::sleep(Duration::from_millis(1000)).await;

    //     let alice_peers = alice.peers().await?;
    //     let bob_peers = bob.peers().await?;
    //     assert!(alice_peers
    //         .iter()
    //         .any(|conn| conn.addr.peer_id == bob_peer_id));
    //     assert!(bob_peers
    //         .iter()
    //         .any(|conn| conn.addr.peer_id == alice_peer_id));

    //     Ok(())
    // }
}
