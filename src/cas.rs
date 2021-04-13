use super::codec::SupportedCodecs;
use libipld::cid::{Cid, Version};
use rocket::tokio::io::AsyncRead;

#[rocket::async_trait]
pub trait ContentAddressedStorage: Send + Sync {
    type Error;
    async fn put<C: AsyncRead + Send + Unpin>(
        &self,
        content: &mut C,
        codec: SupportedCodecs,
    ) -> Result<Cid, Self::Error>;
    async fn get(&self, address: &Cid) -> Result<Option<Vec<u8>>, Self::Error>;
    async fn delete(&self, address: &Cid) -> Result<(), Self::Error>;
}
