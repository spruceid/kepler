use crate::storage::ImmutableStore;
use core::task::Poll;
use either::Either;
use exchange_protocol::RequestResponse;
use futures::{
    future::Pending,
    io::{copy, AsyncRead, AsyncReadExt, AsyncWrite, Error as FIoError, Take},
    task::Context,
};
use libipld::Cid;
use libp2p::{
    autonat::Behaviour as AutoNat,
    core::ConnectedPoint,
    dcutr::Behaviour as Dcutr,
    gossipsub::Behaviour as GossipSub,
    identify::Behaviour as Identify,
    identity::PeerId,
    kad::{
        record::store::{MemoryStore, RecordStore},
        Kademlia,
    },
    ping::Behaviour as Ping,
    relay::client::Behaviour as Client,
    swarm::{
        behaviour::toggle::Toggle, ConnectionHandler, ConnectionHandlerEvent, ConnectionId,
        FromSwarm, KeepAlive, NetworkBehaviour, NetworkBehaviourAction, PollParameters,
        SubstreamProtocol,
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

pub struct Behaviour<KS = MemoryStore>
where
    KS: RecordStore + Send + 'static,
{
    exchange: RequestResponse<KeplerSwap, &'static [u8]>,
    base: BaseBehaviour<KS>,
}

#[derive(NetworkBehaviour, Debug)]
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

pub enum Event {
    NewEpoch,
    GossipSub,
}
pub struct Handler {}
pub enum HandlerError {}
pub enum HandlerInEvent {}
pub enum HandlerOutEvent {}

impl<KS> NetworkBehaviour for Behaviour<KS>
where
    KS: RecordStore + Send + 'static,
{
    type ConnectionHandler =
        Either<Handler, <BaseBehaviour<KS> as NetworkBehaviour>::ConnectionHandler>;
    type OutEvent = Event;

    fn on_swarm_event(&mut self, event: FromSwarm<'_, Self::ConnectionHandler>) {}

    fn on_connection_handler_event(
        &mut self,
        _peer_id: PeerId,
        _connection_id: ConnectionId,
        _event: <Self::ConnectionHandler as ConnectionHandler>::OutEvent,
    ) {
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        params: &mut impl PollParameters,
    ) -> Poll<
        NetworkBehaviourAction<
            Self::OutEvent,
            <Self::ConnectionHandler as ConnectionHandler>::InEvent,
        >,
    > {
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
