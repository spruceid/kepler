use core::time::Duration;
use derive_builder::Builder;
use libp2p::{
    core::{muxing::StreamMuxerBox, transport::Boxed, PeerId},
    dcutr::behaviour::Behaviour as Dcutr,
    gossipsub::{
        Gossipsub, GossipsubConfig, GossipsubConfigBuilder, MessageAuthenticity, ValidationMode,
    },
    identify::{Behaviour as Identify, Config as OIdentifyConfig},
    identity::{Keypair, PublicKey},
    kad::{
        record::store::{MemoryStore, MemoryStoreConfig, RecordStore},
        Kademlia, KademliaConfig,
    },
    ping::{Behaviour as Ping, Config as PingConfig},
    relay::v2::client::Client,
    swarm::{behaviour::toggle::Toggle, Swarm},
    NetworkBehaviour,
};
use thiserror::Error;

const PROTOCOL_VERSION: &'static str = "kepler/0.1.0";

pub type OrbitSwarm<KS = MemoryStore> = Swarm<Behaviour<KS>>;

mod builder {
    use super::*;
    #[derive(Builder, Clone, Debug)]
    #[builder(build_fn(skip), setter(into), name = "BehaviourBuilder", derive(Debug))]
    pub struct BehaviourConfig<KSC = MemoryStoreConfig>
    where
        KSC: Default,
    {
        #[builder(field(type = "IdentifyConfig"))]
        identify: IdentifyConfig,
        #[builder(field(type = "PingConfig"))]
        ping: PingConfig,
        #[builder(field(type = "GossipsubConfig"))]
        gossipsub: GossipsubConfig,
        #[builder(field(type = "KademliaConfig"))]
        kademlia: KademliaConfig,
        #[builder(field(type = "KSC"))]
        kademlia_store: KSC,
    }

    impl<KSC> BehaviourBuilder<KSC>
    where
        KSC: Default,
    {
        pub fn build<KS>(
            self,
            keypair: Keypair,
            relay: Option<Client>,
        ) -> Result<Behaviour<KS>, OrbitBehaviourBuildError>
        where
            KSC: Default + RecordStoreConfig<KS>,
            KS: for<'a> RecordStore<'a> + Send,
        {
            let peer_id = keypair.public().to_peer_id();
            Ok(Behaviour {
                identify: Identify::new(self.identify.to_config(keypair.public())),
                ping: Ping::new(self.ping),
                gossipsub: Gossipsub::new(
                    MessageAuthenticity::Signed(keypair),
                    GossipsubConfigBuilder::from(self.gossipsub)
                        // always ensure validation
                        .validation_mode(ValidationMode::Strict)
                        .build()
                        .map_err(OrbitBehaviourBuildError::Gossipsub)?,
                )
                .map_err(OrbitBehaviourBuildError::Gossipsub)?,
                relay: relay.into(),
                kademlia: Kademlia::with_config(
                    peer_id,
                    self.kademlia_store.init(peer_id),
                    self.kademlia,
                ),
                dcutr: Dcutr::new(),
            })
        }
    }

    #[derive(Error, Debug)]
    pub enum OrbitBehaviourBuildError {
        #[error("{0}")]
        Gossipsub(&'static str),
    }
}
pub use builder::{BehaviourBuilder, OrbitBehaviourBuildError};

#[derive(NetworkBehaviour)]
pub struct Behaviour<KS>
where
    KS: 'static + for<'a> RecordStore<'a> + Send,
{
    identify: Identify,
    ping: Ping,
    gossipsub: Gossipsub,
    relay: Toggle<Client>,
    kademlia: Kademlia<KS>,
    dcutr: Dcutr,
}

pub trait RecordStoreConfig<S>
where
    S: for<'a> RecordStore<'a>,
{
    fn init(self, id: PeerId) -> S;
}

impl RecordStoreConfig<MemoryStore> for MemoryStoreConfig {
    fn init(self, id: PeerId) -> MemoryStore {
        MemoryStore::with_config(id, self)
    }
}

#[derive(Builder, Default, Debug, Clone)]
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
    fn to_config(self, key: PublicKey) -> OIdentifyConfig {
        OIdentifyConfig::new(PROTOCOL_VERSION.to_string(), key)
            .with_initial_delay(self.initial_delay)
            .with_interval(self.interval)
            .with_push_listen_addr_updates(self.push_listen_addr_updates)
            .with_cache_size(self.cache_size)
    }
}

impl From<OIdentifyConfig> for IdentifyConfig {
    fn from(c: OIdentifyConfig) -> Self {
        Self {
            initial_delay: c.initial_delay,
            interval: c.interval,
            push_listen_addr_updates: c.push_listen_addr_updates,
            cache_size: c.cache_size,
        }
    }
}
