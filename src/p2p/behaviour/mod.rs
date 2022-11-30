use crate::storage::ImmutableStore;
use core::task::Poll;
use exchange_protocol::RequestResponseCodec;
use futures::{
    io::{copy, AsyncRead, AsyncReadExt, AsyncWrite, Error as FIoError, Take},
    task::Context,
};
use libipld::Cid;
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
    swarm::{
        behaviour::toggle::Toggle, NetworkBehaviour, NetworkBehaviourAction, PollParameters, Swarm,
    },
};
use std::io::Error as IoError;

pub type OrbitSwarm<KS = MemoryStore> = Swarm<Behaviour<KS>>;
mod builder;
pub mod swap;

pub use builder::{BehaviourConfig, OrbitBehaviourBuildError};

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
    autonat: AutoNat,
}
