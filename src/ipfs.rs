use super::cas::ContentAddressedStorage;
use super::codec::SupportedCodecs;
use super::ipfs_embed::db::StorageService;
use libipld::{
    block::Block,
    cid::{multihash::Code, Cid},
    raw::RawCodec,
    store::DefaultParams,
};

#[rocket::async_trait]
impl ContentAddressedStorage for StorageService<DefaultParams> {
    type Error = anyhow::Error;
    async fn put(&self, content: &[u8], _codec: SupportedCodecs) -> Result<Cid, Self::Error> {
        // TODO find a way to stream this better? (use .take with max block size?)
        // TODO impl support for chunking with linked data (e.g. use IpldCodec)
        let block = Block::<DefaultParams>::encode(RawCodec, Code::Blake3_256, content)?;
        self.insert(&block)?;
        self.alias(&block.cid().to_bytes(), Some(block.cid()))?;
        Ok(*block.cid())
    }
    async fn get(&self, address: &Cid) -> Result<Option<Vec<u8>>, Self::Error> {
        // TODO this api returns Result<Block, anyhow::Error>, with an err thrown for no block found
        // until this API changes (a breaking change), we will error here when no block found
        // TODO now that we have full control we can handle that case nicely
        match self.get(address) {
            Ok(Some(block)) => Ok(Some(block)),
            Ok(None) => Ok(None), // is that right?
            Err(_) => {
                // TODO add network bit, although I'd argue we might not want to trigger the replication which should already be on its way
                // or in other words, separate the HTTP gateway from the IPFS gateway
                // or maybe we want it actually and not have the block store refer to the IPFS gateway
                // self.sync(address, self.peers()).await?;
                self.get(address)
            }
        }
    }
    async fn delete(&self, address: &Cid) -> Result<(), Self::Error> {
        // TODO this does not enforce deletion across the network, we need to devise a method for that via the pubsub stuff
        self.alias(&address.to_bytes(), None)?;
        // TODO
        // self.remove_record(&address.hash().to_bytes().into());
        Ok(())
    }
    async fn list(&self) -> Result<Vec<Cid>, Self::Error> {
        // return a list of all CIDs which are aliased/pinned
        self.iter().map(|i| {
            i.filter(|c| self.reverse_alias(&c).map(|o| o.is_some()).unwrap_or(false))
                .collect()
        })
    }
}
