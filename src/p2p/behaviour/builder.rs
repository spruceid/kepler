use crate::p2p::{behaviour::Behaviour, IdentifyConfig};
use derive_builder::Builder;
use libp2p::{
    autonat::{Behaviour as AutoNat, Config as AutoNatConfig},
    core::PeerId,
    dcutr::behaviour::Behaviour as Dcutr,
    gossipsub::{
        Gossipsub, GossipsubConfig, GossipsubConfigBuilder, MessageAuthenticity, ValidationMode,
    },
    identify::Behaviour as Identify,
    identity::Keypair,
    kad::{
        record::store::{MemoryStore, MemoryStoreConfig, RecordStore},
        Kademlia, KademliaConfig,
    },
    ping::{Behaviour as Ping, Config as PingConfig},
    relay::v2::client::Client,
};
use thiserror::Error;

// we use derive_builder here to make a conveniant builder, but we do not export
// the actual config struct
#[derive(Builder)]
#[builder(build_fn(skip), setter(into), name = "BehaviourConfig", derive(Debug))]
pub struct BehaviourConfigDummy<KSC = MemoryStoreConfig>
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
    #[builder(field(type = "AutoNatConfig"))]
    autonat: AutoNatConfig,
}

impl<KSC> BehaviourConfig<KSC>
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
            autonat: AutoNat::new(peer_id, self.autonat),
        })
    }
}

#[derive(Error, Debug)]
pub enum OrbitBehaviourBuildError {
    #[error("{0}")]
    Gossipsub(&'static str),
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
