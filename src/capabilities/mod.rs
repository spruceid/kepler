pub mod store;

use crate::orbit::AbortOnDrop;
use anyhow::Result;
use ipfs::PeerId;
use libipld::{cbor::DagCborCodec, codec::Decode, multibase::Base};
use rocket::futures::{Stream, StreamExt};
pub use store::AuthRef;
use store::{CapsMessage, Store};

use std::io::Cursor;
use std::sync::Arc;

#[rocket::async_trait]
pub trait Invoke<T> {
    async fn invoke(&self, invocation: &T) -> Result<AuthRef>;
}

#[derive(Clone)]
pub struct Service {
    pub store: Store,
    _task: Arc<AbortOnDrop<()>>,
}

impl std::ops::Deref for Service {
    type Target = Store;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl Service {
    fn new(store: Store, task: AbortOnDrop<()>) -> Self {
        Self {
            store,
            _task: Arc::new(task),
        }
    }
    pub async fn start(store: Store) -> Result<Self> {
        let events = store
            .ipfs
            .pubsub_subscribe(store.id.get_cid().to_string_of_base(Base::Base58Btc)?)
            .await?
            .map(
                |msg| match CapsMessage::decode(DagCborCodec, &mut Cursor::new(&msg.data)) {
                    Ok(m) => Ok((msg.source, m)),
                    Err(e) => Err(anyhow!(e)),
                },
            );
        let peer_id = store.ipfs.identity().await?.0.to_peer_id();
        let task = AbortOnDrop::new(tokio::spawn(caps_task(events, store.clone(), peer_id)));
        store.request_heads().await?;
        Ok(Service::new(store, task))
    }
}

async fn caps_task(
    events: impl Stream<Item = Result<(PeerId, CapsMessage)>> + Send,
    store: Store,
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
                    if let Err(e) = match store
                        .ipfs
                        .get_block(&cid)
                        .await
                        .and_then(|b| b.decode())
                        .map(|i| store.apply_invocations(i))
                    {
                        Ok(f) => f.await,
                        Err(e) => Err(e),
                    } {
                        debug!("failed to apply recieved invocation {}", e);
                    }
                }
                Ok((_, CapsMessage::Update(update))) => {
                    debug!("recieved updates");
                    if let Err(e) = store.apply(update).await {
                        debug!("failed to apply recieved updates {}", e);
                    }
                }
                Ok((_, CapsMessage::StateReq)) => {
                    if let Err(e) = store.broadcast_heads().await {
                        debug!(
                            "failed to broadcast updates in response to state request {}",
                            e
                        );
                    }
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
