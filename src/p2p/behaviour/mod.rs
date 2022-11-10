use libp2p::{
    autonat::Behaviour as AutoNat,
    dcutr::behaviour::Behaviour as Dcutr,
    gossipsub::Gossipsub,
    identify::Behaviour as Identify,
    kad::{
        record::store::{MemoryStore, RecordStore},
        Kademlia,
    },
    ping::Behaviour as Ping,
    relay::v2::client::Client,
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour, Swarm},
};

pub type OrbitSwarm<KS = MemoryStore> = Swarm<Behaviour<KS>>;
mod builder;

pub use builder::{BehaviourBuilder, OrbitBehaviourBuildError};

#[derive(NetworkBehaviour)]
pub struct Behaviour<KS>
where
    KS: RecordStore + Send,
{
    identify: Identify,
    ping: Ping,
    gossipsub: Gossipsub,
    relay: Toggle<Client>,
    kademlia: Kademlia<KS>,
    dcutr: Dcutr,
    autonat: AutoNat,
}
