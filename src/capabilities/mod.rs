pub mod store;

use crate::{heads::HeadStore, resource::OrbitId};
use anyhow::Result;
use std::str::FromStr;
pub use store::AuthRef;
use store::Store;

#[rocket::async_trait]
pub trait Invoke<T> {
    async fn invoke(&self, invocation: &T) -> Result<AuthRef>;
}

#[derive(Clone)]
pub struct Service<H> {
    pub store: Store<H>,
}

impl<H> std::ops::Deref for Service<H> {
    type Target = Store<H>;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl<H> Service<H>
where
    H: 'static + HeadStore + Send + Sync + Clone,
{
    pub async fn start(store: Store<H>) -> Result<Self> {
        // let id = OrbitId::from_str(&String::from_utf8(store.id)?)?
        //     .get_cid()
        //     .to_string();
        // let events = store.ipfs.pubsub_subscribe(id).await?.map(|msg| {
        //     match bincode::deserialize(&msg.data) {
        //         Ok(kv_msg) => Ok((msg.source, kv_msg)),
        //         Err(e) => Err(anyhow!(e)),
        //     }
        // });
        // let peer_id = store.ipfs.identity().await?.0.to_peer_id();
        // let task = tokio::spawn(kv_task(events, store.clone(), peer_id));
        // store.request_heads().await?;
        // Ok(Service::new(store, task))
        Ok(Service { store })
    }
}
