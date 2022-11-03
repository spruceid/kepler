pub mod store;

use anyhow::Result;
use kepler_lib::libipld::Cid;
use store::Store;

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

impl<B> Service<B> {
    fn new(store: Store<B>) -> Self {
        Self { store }
    }
    pub async fn start(store: Store<B>) -> Result<Self> {
        Ok(Service::new(store))
    }
}
