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
    relay::{new_transport_and_behaviour, Relay},
    swarm::{Swarm, NetworkBehaviourEventProcess},
    tcp::TokioTcpConfig as TcpConfig,
    yamux::YamuxConfig,
    ping::{Ping, PingEvent},
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

#[derive(libp2p::NetworkBehaviour)]
struct RelayBehaviour {
    relay: Relay,
    ping: Ping,
}

impl NetworkBehaviourEventProcess<()> for RelayBehaviour {
    fn inject_event(&mut self, _event: ()) {}
}
impl NetworkBehaviourEventProcess<PingEvent> for RelayBehaviour {
    fn inject_event(&mut self, _event: PingEvent) {}
}

impl RelayNode {
    pub fn new(port: u16, key: Keypair) -> Result<Self> {
        let local_public_key = key.public();
        let id = local_public_key.into_peer_id();
        let base = MemoryTransport.or_transport(DnsConfig::system(TcpConfig::new().nodelay(true))?);
        let (t, r) = new_transport_and_behaviour(Default::default(), base);
        let b = RelayBehaviour {
            relay: r,
            ping: Ping::default()
        };

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
        let mut swarm = Swarm::new(transport, b, id);

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
    use ipfs::{IpfsOptions, UninitializedIpfs, TestTypes, MultiaddrWithPeerId, MultiaddrWithoutPeerId};
    use libp2p::core::multiaddr::{multiaddr, Protocol};
    use std::convert::TryFrom;


    #[tokio::test(flavor = "multi_thread")]
    async fn relay() -> Result<()> {
        crate::tracing_try_init();
        let relay = RelayNode::new(10000, Keypair::generate_ed25519())?;

        let mut alice_opts = IpfsOptions::inmemory_with_generated_keys();
        alice_opts.listening_addrs = vec![multiaddr!(P2pCircuit)];
        let (alice, task) = UninitializedIpfs::<TestTypes>::new(alice_opts).start().await?;
        tokio::spawn(task);
        let alice_peer_id = alice.identity().await?.0.into_peer_id();

        let mut bob_opts = IpfsOptions::inmemory_with_generated_keys();
        bob_opts.listening_addrs = vec![multiaddr!(Ip4([127,0,0,1]), Tcp(10001u16))];
        let (bob, task) = UninitializedIpfs::<TestTypes>::new(bob_opts).start().await?;
        tokio::spawn(task);
        let bob_peer_id = bob.identity().await?.0.into_peer_id();

        alice.connect(
            MultiaddrWithoutPeerId::try_from(relay.internal())?.with(relay.id.clone())
        ).await.expect("alice failed to connect to relay");


        bob.connect(
        MultiaddrWithPeerId::try_from(
                relay.external()
                    .with(Protocol::P2p(relay.id.clone().into()))
                    .with(Protocol::P2pCircuit)
                    .with(Protocol::P2p(alice_peer_id.clone().into())))?
        ).await.expect("bob failed to connect to alice");

        tokio::time::sleep(Duration::from_millis(1000)).await;

        let alice_peers = alice.peers().await?;
        let bob_peers = bob.peers().await?;
        assert!(alice_peers.iter().any(|conn| conn.addr.peer_id == bob_peer_id));
        assert!(bob_peers.iter().any(|conn| conn.addr.peer_id == alice_peer_id));

        Ok(())
    }
}
