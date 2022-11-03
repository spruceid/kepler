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
        record::store::{MemoryStore, MemoryStoreConfig, RecordStore},
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
pub struct OrbitNodeConfig<KSC = MemoryStoreConfig>
where
    KSC: Default,
{
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
    kademlia_store: KSC,
}

#[derive(NetworkBehaviour)]
pub struct OrbitNode<KS>
where
    KS: 'static + for<'a> RecordStore<'a> + Send,
{
    identify: Identify,
    ping: Ping,
    gossipsub: Gossipsub,
    relay: Toggle<Client>,
    kademlia: Kademlia<KS>,
    dcutr: DcutrBehaviour,
}

#[derive(Error, Debug)]
pub enum OrbitNodeInitError {
    #[error("{0}")]
    Gossipsub(&'static str),
}

impl<KS> OrbitNode<KS>
where
    KS: 'static + for<'a> RecordStore<'a> + Send,
{
    pub fn new<KSC>(c: OrbitNodeConfig<KSC>) -> Result<Self, OrbitNodeInitError>
    where
        KSC: RecordStoreConfig<KS> + Default,
    {
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
            relay: None.into(),
            kademlia: Kademlia::with_config(peer_id, c.kademlia_store.init(peer_id), c.kademlia),
            dcutr: DcutrBehaviour::new(),
        })
    }

    pub fn new_with_relay<KSC>(
        c: OrbitNodeConfig<KSC>,
        relay_client: Client,
    ) -> Result<Self, OrbitNodeInitError>
    where
        KSC: RecordStoreConfig<KS> + Default,
    {
        Ok(Self {
            relay: Some(relay_client).into(),
            ..Self::new(c)?
        })
    }
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
