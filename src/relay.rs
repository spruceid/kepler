use anyhow::Result;
use libp2p::{
    core::{
        identity::Keypair,
        multiaddr::multiaddr,
        transport::MemoryTransport,
        upgrade::{SelectUpgrade, Version},
        Multiaddr, PeerId, Transport,
    },
    dns::TokioDnsConfig as DnsConfig,
    mplex::MplexConfig,
    noise::{self, NoiseConfig, X25519Spec},
    relay::new_transport_and_behaviour,
    swarm::Swarm,
    tcp::TokioTcpConfig as TcpConfig,
    yamux::YamuxConfig,
};
use rocket::{
    futures::stream::StreamExt,
    tokio::{spawn, task::JoinHandle},
};
use std::time::Duration;

pub struct RelayNode {
    pub port: u16,
    pub id: PeerId,
    task: JoinHandle<()>,
}

impl RelayNode {
    pub fn new(port: u16, key: Keypair) -> Result<Self> {
        let local_public_key = key.public();
        let id = local_public_key.into_peer_id();
        let base = MemoryTransport.or_transport(DnsConfig::system(TcpConfig::new().nodelay(true))?);
        let (t, r) = new_transport_and_behaviour(Default::default(), base);

        let transport = t
            .upgrade(Version::V1)
            .authenticate(
                NoiseConfig::xx(noise::Keypair::<X25519Spec>::new().into_authentic(&key)?)
                    .into_authenticated(),
            )
            .multiplex(SelectUpgrade::new(
                YamuxConfig::default(),
                MplexConfig::new(),
            ))
            .timeout(Duration::from_secs(5))
            .boxed();

        let relay_tcp_addr = multiaddr!(Ip4([127, 0, 0, 1]), Tcp(port));
        let relay_mem_addr = multiaddr!(Memory(port));
        let mut swarm = Swarm::new(transport, r, id);

        tracing::debug!(
            "opened relay: {} at {}, {}",
            id,
            relay_mem_addr,
            relay_tcp_addr
        );

        swarm.listen_on(relay_tcp_addr)?;
        swarm.listen_on(relay_mem_addr)?;

        let task = spawn(swarm.for_each_concurrent(None, |_| async move {}));
        Ok(Self { port, task, id })
    }

    pub fn internal(&self) -> Multiaddr {
        multiaddr!(Memory(self.port))
    }

    pub fn external(&self) -> Multiaddr {
        multiaddr!(Ip4([127, 0, 0, 1]), Tcp(self.port))
    }
}

impl Drop for RelayNode {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use ipfs_embed::{generate_keypair, Config, DefaultParams, Ipfs as EIpfs, ToLibp2p};
    use libp2p::core::multiaddr::{multiaddr, Protocol};
    use std::path::Path;
    use tempdir::TempDir;

    type Ipfs = EIpfs<DefaultParams>;

    fn get_cfg<P: AsRef<Path>>(path: P) -> Config {
        let mut c = Config::new(path.as_ref(), generate_keypair());
        // ensure mdns isnt doing all the work here
        c.network.mdns = None;
        c
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn relay() -> Result<()> {
        crate::tracing_try_init();
        let relay = RelayNode::new(10000, generate_keypair().to_keypair())?;
        let tmp = TempDir::new("test")?;

        let alice = Ipfs::new(get_cfg(tmp.path().join("alice"))).await?;
        let bob = Ipfs::new(get_cfg(tmp.path().join("bob"))).await?;

        alice.listen_on(multiaddr!(P2pCircuit))?.next().await;
        alice.dial_address(&relay.id, relay.internal());

        bob.listen_on(multiaddr!(Ip4([127u8, 0u8, 0u8, 1u8]), Tcp(10001u16)))?
            .next()
            .await;
        tracing::debug!("dialing alice");
        bob.dial_address(
            &alice.local_peer_id(),
            relay
                .external()
                .with(Protocol::P2p(relay.id.clone().into()))
                .with(Protocol::P2pCircuit)
                .with(Protocol::P2p(alice.local_peer_id().into())),
        );

        std::thread::sleep(Duration::from_millis(1000));

        assert!(alice.is_connected(&bob.local_peer_id()));
        assert!(bob.is_connected(&alice.local_peer_id()));

        Ok(())
    }
}
