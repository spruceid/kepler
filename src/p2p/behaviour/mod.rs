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
mod builder;

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
