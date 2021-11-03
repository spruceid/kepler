use super::cas::ContentAddressedStorage;
use super::codec::SupportedCodecs;
use ipfs_embed::Ipfs as OIpfs;
use libipld::{
    block::Block as OBlock,
    cid::{multihash::Code, Cid},
    raw::RawCodec,
    store::StoreParams,
    IpldCodec,
};

#[derive(Clone, Debug, Default)]
pub struct KeplerParams;

impl StoreParams for KeplerParams {
    const MAX_BLOCK_SIZE: usize = 10_485_760;
    type Codecs = IpldCodec;
    type Hashes = Code;
}

pub type Ipfs = OIpfs<KeplerParams>;
pub type Block = OBlock<KeplerParams>;

#[rocket::async_trait]
impl ContentAddressedStorage for Ipfs {
    type Error = anyhow::Error;
    async fn put(&self, content: &[u8], _codec: SupportedCodecs) -> Result<Cid, Self::Error> {
        // TODO find a way to stream this better? (use .take with max block size?)
        // TODO impl support for chunking with linked data (e.g. use IpldCodec)
        let block = Block::encode(RawCodec, Code::Blake3_256, content)?;
        self.insert(&block)?;
        self.alias(block.cid().to_bytes(), Some(block.cid()))?;
        Ok(*block.cid())
    }
    async fn get(&self, address: &Cid) -> Result<Option<Vec<u8>>, Self::Error> {
        // TODO this api returns Result<Block, anyhow::Error>, with an err thrown for no block found
        // until this API changes (a breaking change), we will error here when no block found
        Ok(Some(self.get(address)?.data().to_vec()))
    }
    async fn delete(&self, address: &Cid) -> Result<(), Self::Error> {
        // TODO this does not enforce deletion across the network, we need to devise a method for that via the pubsub stuff
        self.alias(address.to_bytes(), None)?;
        self.remove_record(&address.hash().to_bytes().into());
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
