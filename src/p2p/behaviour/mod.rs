use libp2p::{
    autonat::Behaviour as AutoNat,
    dcutr::Behaviour as Dcutr,
    gossipsub::Behaviour as GossipSub,
    identify::Behaviour as Identify,
    kad::{record::store::RecordStore, Kademlia},
    ping::Behaviour as Ping,
    relay::client::Behaviour as Client,
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour, Swarm},
};

mod builder;

pub use builder::{BehaviourConfig, OrbitBehaviourBuildError};

// TODO impl network behaviour
// this is temporary as a checkpoint
#[derive(NetworkBehaviour)]
pub struct Behaviour<KS>
where
    KS: RecordStore + Send + 'static,
{
    base: BaseBehaviour<KS>,
}

#[derive(NetworkBehaviour)]
pub struct BaseBehaviour<KS>
where
    KS: RecordStore + Send + 'static,
{
    identify: Identify,
    ping: Ping,
    gossipsub: GossipSub,
    relay: Toggle<Client>,
    kademlia: Kademlia<KS>,
    dcutr: Dcutr,
    autonat: AutoNat,
}

async fn poll_swarm<KS>(_swarm: Swarm<Behaviour<KS>>) -> Result<(), ()>
where
    KS: RecordStore + Send + 'static,
{
    Ok(())
}
