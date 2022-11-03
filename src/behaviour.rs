use core::time::Duration;
use derive_builder::Builder;
use libp2p::{
    core::PeerId,
    dcutr::behaviour::Behaviour as DcutrBehaviour,
    gossipsub::{
        Gossipsub, GossipsubConfig, GossipsubConfigBuilder, MessageAuthenticity, ValidationMode,
    },
    identify::{Behaviour as Identify, Config as OIdentifyConfig},
    identity::{Keypair, PublicKey},
    kad::{
        record::store::{MemoryStore, MemoryStoreConfig},
        Kademlia, KademliaConfig,
    },
    ping::{Behaviour as Ping, Config as PingConfig},
    relay::v2::client::Client,
    swarm::behaviour::toggle::Toggle,
    NetworkBehaviour,
};
use thiserror::Error;

const PROTOCOL_VERSION: &'static str = "kepler/0.1.0";

#[derive(Builder, Default, Debug)]
pub struct IdentifyConfig {
    #[builder(setter(into), default = "Duration::from_millis(500)")]
    initial_delay: Duration,
    #[builder(setter(into), default = "Duration::from_secs(300)")]
    interval: Duration,
    #[builder(setter(into), default = "false")]
    push_listen_addr_updates: bool,
    #[builder(setter(into), default = "0")]
    cache_size: usize,
}

impl IdentifyConfig {
    pub fn to_config(self, key: PublicKey) -> OIdentifyConfig {
        OIdentifyConfig::new(PROTOCOL_VERSION.to_string(), key)
            .with_initial_delay(self.initial_delay)
            .with_interval(self.interval)
            .with_push_listen_addr_updates(self.push_listen_addr_updates)
            .with_cache_size(self.cache_size)
    }
}

#[derive(Builder)]
#[builder(pattern = "owned")]
pub struct OrbitNodeConfig {
    #[builder(setter(into))]
    identity: Keypair,
    #[builder(setter(into), default)]
    identify: IdentifyConfig,
    #[builder(setter(into), default)]
    ping: PingConfig,
    #[builder(setter(into), default)]
    gossipsub: GossipsubConfig,
    #[builder(setter(into), default)]
    kademlia: KademliaConfig,
    #[builder(setter(into), default)]
    kademlia_store: MemoryStoreConfig,
}

#[derive(NetworkBehaviour)]
pub struct OrbitNode {
    identify: Identify,
    ping: Ping,
    gossipsub: Gossipsub,
    relay: Toggle<Client>,
    kademlia: Kademlia<MemoryStore>,
    dcutr: DcutrBehaviour,
}

#[derive(Error, Debug)]
pub enum OrbitNodeInitError {
    #[error("{0}")]
    Gossipsub(&'static str),
}

impl OrbitNode {
    pub fn new(c: OrbitNodeConfig) -> Result<Self, OrbitNodeInitError> {
        let peer_id = c.identity.public().to_peer_id();
        Ok(Self {
            identify: Identify::new(c.identify.to_config(c.identity.public())),
            ping: Ping::new(c.ping),
            gossipsub: Gossipsub::new(
                MessageAuthenticity::Signed(c.identity),
                GossipsubConfigBuilder::from(c.gossipsub)
                    // always ensure validation
                    .validation_mode(ValidationMode::Strict)
                    .build()
                    .map_err(OrbitNodeInitError::Gossipsub)?,
            )
            .map_err(OrbitNodeInitError::Gossipsub)?,
            kademlia: Kademlia::with_config(
                peer_id,
                MemoryStore::with_config(peer_id, c.kademlia_store),
                c.kademlia,
            ),
            relay: None.into(),
            dcutr: DcutrBehaviour::new(),
        })
    }

    pub fn new_with_relay(
        c: OrbitNodeConfig,
        relay_client: Client,
    ) -> Result<Self, OrbitNodeInitError> {
        Ok(Self {
            relay: Some(relay_client).into(),
            ..Self::new(c)?
        })
    }
}
