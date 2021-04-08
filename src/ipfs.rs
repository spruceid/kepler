use super::cas::ContentAddressedStorage;
use super::codec::SupportedCodecs;
use ipfs_embed::Ipfs;
use libipld::{
    block::Block,
    cid::{
        multihash::{Code, MultihashDigest},
        Cid,
    },
    raw::RawCodec,
    store::DefaultParams,
};
use rocket::tokio::io::{AsyncRead, AsyncReadExt};

const MAX_BLOCK_SIZE: usize = 1024 * 1024 * 4;

#[rocket::async_trait]
impl ContentAddressedStorage for Ipfs<DefaultParams> {
    type Error = anyhow::Error;
    async fn put<C: AsyncRead + Send + Unpin>(
        &self,
        content: &mut C,
        codec: SupportedCodecs,
    ) -> Result<Cid, Self::Error> {
        todo!()
    }
    async fn get(&self, address: Cid) -> Result<Option<Vec<u8>>, Self::Error> {
        todo!()
    }
    async fn delete(&self, address: Cid) -> Result<(), Self::Error> {
        todo!()
    }
}
