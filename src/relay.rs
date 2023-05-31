use anyhow::Result;
use libp2p::{
    core::{multiaddr::multiaddr, Multiaddr},
    identity::{Keypair, PeerId},
};

pub struct RelayNode {
    pub port: u16,
    pub id: PeerId,
}

impl RelayNode {
    pub async fn new(port: u16, keypair: Keypair) -> Result<Self> {
        let local_public_key = keypair.public();
        let id = local_public_key.to_peer_id();

        Ok(Self { port, id })
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
pub mod test {}
