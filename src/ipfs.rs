use super::cas::ContentAddressedStorage;
use super::codec::SupportedCodecs;
use ipfs_embed::{Ipfs, Key};
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
        // TODO find a way to stream this better? (use .take with max block size?)
        let mut buf = Vec::<u8>::new();
        content.read_to_end(&mut buf).await?;
        // TODO impl support for chunking with linked data (e.g. use IpldCodec)
        let block = Block::<DefaultParams>::encode(RawCodec, Code::Blake3_256, &buf)?;
        self.insert(&block)?.await?;
        Ok(*block.cid())
    }
    async fn get(&self, address: &Cid) -> Result<Option<Vec<u8>>, Self::Error> {
        // TODO this api returns Result<Block, anyhow::Error>, with an err thrown for no block found
        // until this API changes (a breaking change), we will error here when no block found
        Ok(Some(self.get(address)?.data().to_vec()))
    }
    async fn delete(&self, address: &Cid) -> Result<(), Self::Error> {
        // TODO this does not enforce deletion across the network, we need to devise a method for that via the pubsub stuff
        Ok(self.remove_record(&address.hash().to_bytes().into()))
    }
}
