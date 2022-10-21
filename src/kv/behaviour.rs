use std::{
    sync::{
        mpsc::{Receiver, SyncSender},
        Arc,
    },
    task::{Context, Poll},
};

use libp2p::{
    core::{connection::ConnectionId, ConnectedPoint, Multiaddr, PeerId},
    swarm::{
        handler::DummyConnectionHandler, IntoConnectionHandler, NetworkBehaviour,
        NetworkBehaviourAction, PollParameters,
    },
};
use void::Void;

use crate::orbit::AbortOnDrop;

use super::Store;

#[derive(Clone, Debug)]
pub struct Behaviour {
    sender: SyncSender<Event>,
}

impl Behaviour {
    pub fn new(sender: SyncSender<Event>) -> Self {
        Self { sender }
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = DummyConnectionHandler;

    type OutEvent = ();

    fn new_handler(&mut self) -> Self::ConnectionHandler {
        DummyConnectionHandler::default()
    }

    fn inject_event(&mut self, _peer_id: PeerId, _connection: ConnectionId, _event: Void) {}

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<(), Self::ConnectionHandler, Void>> {
        Poll::Pending
    }

    fn inject_connection_established(
        &mut self,
        peer_id: &PeerId,
        _connection: &ConnectionId,
        _endpoint: &ConnectedPoint,
        _failed_addresses: Option<&Vec<Multiaddr>>,
        _other_established: usize,
    ) {
        if let Err(_e) = self.sender.send(Event::ConnectionEstablished(*peer_id)) {
            tracing::error!("Behaviour process has shutdown.")
        }
    }

    fn inject_connection_closed(
        &mut self,
        peer_id: &PeerId,
        _connection: &ConnectionId,
        _endpoint: &ConnectedPoint,
        _handler: <Self::ConnectionHandler as IntoConnectionHandler>::Handler,
        _remaining_established: usize,
    ) {
        if let Err(_e) = self.sender.send(Event::ConnectionTerminated(*peer_id)) {
            tracing::error!("Behaviour process has shutdown.")
        }
    }
}

#[derive(Clone, Debug)]
pub struct BehaviourProcess(Arc<AbortOnDrop<()>>);

impl BehaviourProcess {
    pub fn new(store: Store, mut receiver: Receiver<Event>) -> Self {
        Self(Arc::new(AbortOnDrop::new(tokio::spawn(async move {
            while let Ok(Ok((event, returned_receiver))) =
                tokio::task::spawn_blocking(move || receiver.recv().map(|ev| (ev, receiver))).await
            {
                receiver = returned_receiver;
                match event {
                    Event::ConnectionEstablished(peer_id) => {
                        if let Err(e) = store.ipfs.pubsub_add_peer(peer_id).await {
                            tracing::error!("failed to add new peer to allowed pubsub peers: {}", e)
                        }
                        if let Err(e) = store.request_heads().await {
                            tracing::error!("failed to request heads from peers: {}", e)
                        }
                    }
                    Event::ConnectionTerminated(peer_id) => {
                        if let Err(e) = store.ipfs.pubsub_remove_peer(peer_id).await {
                            tracing::error!(
                                "failed to remove disconnected peer from allowed pubsub peers: {}",
                                e
                            )
                        }
                    }
                }
            }
        }))))
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    ConnectionEstablished(PeerId),
    ConnectionTerminated(PeerId),
}
