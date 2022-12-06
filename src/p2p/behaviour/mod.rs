use crate::storage::ImmutableStore;
use core::task::Poll;
use exchange_protocol::RequestResponse;
use futures::{
    future::Pending,
    io::{copy, AsyncRead, AsyncReadExt, AsyncWrite, Error as FIoError, Take},
    task::Context,
};
use libipld::Cid;
use libp2p::{
    autonat::Behaviour as AutoNat,
    core::{ConnectedPoint, PeerId},
    dcutr::behaviour::Behaviour as Dcutr,
    gossipsub::Gossipsub,
    identify::Behaviour as Identify,
    kad::{
        record::store::{MemoryStore, RecordStore},
        Kademlia,
    },
    ping::Behaviour as Ping,
    relay::v2::client::Client,
    simple::SimpleProtocol,
    swarm::{
        behaviour::toggle::Toggle, ConnectionHandler, ConnectionHandlerEvent,
        IntoConnectionHandler, KeepAlive, NetworkBehaviour, NetworkBehaviourAction, PollParameters,
        SubstreamProtocol, Swarm,
    },
};
use std::io::Error as IoError;

mod builder;
mod swap;

pub use builder::{BehaviourConfig, OrbitBehaviourBuildError};
pub use swap::KeplerSwap;

pub struct Config {
    commit_epoch: (),
}

pub struct Behaviour {
    config: Config,
    exchange: RequestResponse<KeplerSwap, &'static [u8]>,
}

/// An Epoch is a builder for a block in the capabilities graph.
pub struct Epoch {
    invocations: Vec<()>,
    delegations: Vec<()>,
    revocations: Vec<()>,
}

impl Behaviour {
    pub fn new_epoch(&self) -> Epoch {
        Epoch {
            invocations: vec![],
            delegations: vec![],
            revocations: vec![],
        }
    }

    pub fn submit_epoch(&mut self, epoch: Epoch) -> Result<Cid, ()> {
        todo!()
    }

    fn process_event(&mut self, event: ()) -> Result<(), ()> {
        todo!()
    }
}

pub enum Event {}
pub struct Handler {}
pub struct IntoHandler {}
pub enum HandlerError {}
pub enum HandlerInEvent {}
pub enum HandlerOutEvent {}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = IntoHandler;
    type OutEvent = Event;

    fn new_handler(&mut self) -> Self::ConnectionHandler {
        IntoHandler {}
    }
    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        params: &mut impl PollParameters
    ) -> Poll<NetworkBehaviourAction<Self::OutEvent, Self::ConnectionHandler, <<Self::ConnectionHandler as IntoConnectionHandler>::Handler as ConnectionHandler>::InEvent>>{
        Poll::Pending
    }
}

impl ConnectionHandler for Handler {
    type InEvent = HandlerInEvent;
    type OutEvent = HandlerOutEvent;
    type Error = HandlerError;
    type InboundProtocol = SimpleProtocol<fn() -> Pending<Result<(), ()>>>;
    type OutboundProtocol = SimpleProtocol<fn() -> Pending<Result<(), ()>>>;
    type InboundOpenInfo = SimpleProtocol<fn() -> Pending<Result<(), ()>>>;
    type OutboundOpenInfo = SimpleProtocol<fn() -> Pending<Result<(), ()>>>;

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(SimpleProtocol::new([], || async {}), ())
    }
    fn connection_keep_alive(&self) -> KeepAlive {
        KeepAlive::Yes
    }
    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<
        ConnectionHandlerEvent<
            Self::OutboundProtocol,
            Self::OutboundOpenInfo,
            Self::OutEvent,
            Self::Error,
        >,
    > {
        Poll::Pending
    }
}

impl IntoConnectionHandler for IntoHandler {
    type Handler = Handler;

    fn into_handler(
        self,
        remote_peer_id: &PeerId,
        connected_point: &ConnectedPoint,
    ) -> Self::Handler {
        Handler {}
    }
    fn inbound_protocol(&self) -> <Self::Handler as ConnectionHandler>::InboundProtocol {
        SimpleProtocol::new([], || async {})
    }
}
