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
