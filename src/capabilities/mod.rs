pub mod store;

use crate::orbit::AbortOnDrop;
use anyhow::Result;
use ipfs::PeerId;
use kepler_lib::libipld::{cbor::DagCborCodec, codec::Decode, multibase::Base, Cid};
use rocket::futures::{Stream, StreamExt};
use store::{CapsMessage, Store};

use std::io::Cursor;
use std::sync::Arc;

#[rocket::async_trait]
pub trait Invoke<T> {
    async fn invoke(&self, invocation: &T) -> Result<Cid>;
}

#[derive(Clone)]
pub struct Service<B> {
    pub store: Store<B>,
}

impl<B> std::ops::Deref for Service<B> {
    type Target = Store<B>;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl Service<B> {
    fn new(store: Store<B>) -> Self {
        Self { store }
    }
    pub async fn start(store: Store<B>) -> Result<Self> {
        Ok(Service::new(store))
    }
}

async fn caps_task<B>(
    events: impl Stream<Item = Result<(PeerId, CapsMessage)>> + Send,
    store: Store<B>,
    peer_id: PeerId,
) {
    debug!("starting caps task");
    events
        .for_each_concurrent(None, |ev| async {
            match ev {
                Ok((p, ev)) if p == peer_id => {
                    debug!("{} filtered out this event from self: {:?}", p, ev)
                }
                Ok((_, CapsMessage::Invocation(cid))) => {
                    debug!("recieved invocation");
                    if let Err(e) = store.try_merge_invocations([cid].into_iter()).await {
                        debug!("failed to apply recieved invocation {}", e);
                    }
                }
                Ok((_, CapsMessage::StateReq)) => {
                    // if let Err(e) = store.broadcast_heads().await {
                    //     debug!(
                    //         "failed to broadcast updates in response to state request {}",
                    //         e
                    //     );
                    // }
                }
                Ok((
                    _,
                    CapsMessage::Heads {
                        updates,
                        invocations,
                    },
                )) => {
                    if let Err(e) = store
                        .try_merge_heads(updates.into_iter(), invocations.into_iter())
                        .await
                    {
                        debug!("failed to merge heads {}", e);
                    }
                }
                Err(e) => {
                    debug!("cap service task error {}", e);
                }
            }
        })
        .await;
}
