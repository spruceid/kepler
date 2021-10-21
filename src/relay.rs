use anyhow::Result;
use ipfs_embed::{Config, DefaultParams, Ipfs, Keypair};
use libp2p::core::{multiaddr::multiaddr, Multiaddr, PeerId};
use rocket::futures::stream::StreamExt;
use std::path::Path;

pub struct RelayNode {
    pub port: u16,
    node: Ipfs<DefaultParams>,
}

impl RelayNode {
    pub async fn new(port: u16, key: Keypair) -> Result<Self> {
        let mut cfg = Config::new(Path::new(""), key);
        cfg.storage.path = None;
        cfg.network.bitswap = None;
        cfg.network.streams = None;

        let node = Ipfs::<DefaultParams>::new(cfg).await?;

        let relay_tcp_addr = multiaddr!(Ip4([127, 0, 0, 1]), Tcp(port));
        let relay_mem_addr = multiaddr!(Memory(port));

        tracing::debug!(
            "opened relay: {} at {}, {}",
            node.local_peer_id(),
            relay_mem_addr,
            relay_tcp_addr
        );

        node.listen_on(relay_tcp_addr)?.next().await;
        node.listen_on(relay_mem_addr)?.next().await;

        Ok(Self { port, node })
    }

    pub fn id(&self) -> PeerId {
        self.node.local_peer_id()
    }

    pub fn internal(&self) -> Multiaddr {
        multiaddr!(Memory(self.port))
    }

    pub fn external(&self) -> Multiaddr {
        multiaddr!(Ip4([127, 0, 0, 1]), Tcp(self.port))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use ipfs_embed::{generate_keypair, Config, DefaultParams, Ipfs as EIpfs};
    use libp2p::core::multiaddr::{multiaddr, Protocol};
    use std::path::Path;
    use std::time::Duration;
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
        let relay = RelayNode::new(10000, generate_keypair()).await?;
        let tmp = TempDir::new("test")?;

        let alice = Ipfs::new(get_cfg(tmp.path().join("alice"))).await?;
        let bob = Ipfs::new(get_cfg(tmp.path().join("bob"))).await?;

        alice.listen_on(multiaddr!(P2pCircuit))?.next().await;
        alice.dial_address(&relay.id(), relay.internal());

        bob.listen_on(multiaddr!(Ip4([127u8, 0u8, 0u8, 1u8]), Tcp(10001u16)))?
            .next()
            .await;
        tracing::debug!("dialing alice");
        bob.dial_address(
            &alice.local_peer_id(),
            relay
                .external()
                .with(Protocol::P2p(relay.id().into()))
                .with(Protocol::P2pCircuit)
                .with(Protocol::P2p(alice.local_peer_id().into())),
        );

        std::thread::sleep(Duration::from_millis(1000));

        assert!(alice.is_connected(&bob.local_peer_id()));
        assert!(bob.is_connected(&alice.local_peer_id()));

        Ok(())
    }
}
