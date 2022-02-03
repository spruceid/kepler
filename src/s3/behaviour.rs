use std::{
    sync::{
        mpsc::{Receiver, SyncSender},
        Arc,
    },
    task::{Context, Poll},
};

use ipfs::{Multiaddr, PeerId};
use libp2p::{
    core::connection::ConnectionId,
    swarm::{
        protocols_handler::DummyProtocolsHandler, NetworkBehaviour, NetworkBehaviourAction,
        PollParameters,
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
    type ProtocolsHandler = DummyProtocolsHandler;

    type OutEvent = ();

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        DummyProtocolsHandler::default()
    }

    fn addresses_of_peer(&mut self, _peer_id: &PeerId) -> Vec<Multiaddr> {
        Vec::new()
    }

    fn inject_connected(&mut self, _peer_id: &PeerId) {
        if let Err(_e) = self.sender.send(Event::ConnectionEstablished) {
            tracing::error!("Behaviour process has shutdown.")
        }
    }

    fn inject_disconnected(&mut self, _peer_id: &PeerId) {}

    fn inject_event(&mut self, _peer_id: PeerId, _connection: ConnectionId, _event: Void) {}

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<Void, ()>> {
        Poll::Pending
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
                    Event::ConnectionEstablished => {
                        if let Err(e) = store.request_heads().await {
                            tracing::error!("failed to request heads from peers: {}", e)
                        }
                    }
                }
            }
        }))))
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    ConnectionEstablished,
}
