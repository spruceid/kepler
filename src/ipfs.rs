use super::cas::ContentAddressedStorage;
use super::codec::SupportedCodecs;
//use ipfs_embed::{DefaultParams, Ipfs as OIpfs};
use ipfs::{Ipfs as OIpfs, Types, IpfsPath, path::PathRoot, PinMode, p2p::TSwarm};
use libipld::{
    Ipld,
    block::Block as OBlock,
    cid::{multihash::Code, Cid},
    raw::RawCodec,
    store::{DefaultParams, StoreParams}
};
use libp2p::futures::{StreamExt, TryStreamExt};

pub type KeplerParams = DefaultParams;
// #[derive(Clone, Debug, Default)]
// pub struct KeplerParams;

// impl StoreParams for KeplerParams {
//     const MAX_BLOCK_SIZE: usize = 10_485_760;
//     type Codecs = IpldCodec;
//     type Hashes = Code;
// }

pub type Ipfs = OIpfs<Types>;
pub type Block = OBlock<KeplerParams>;
pub type Swarm = TSwarm<Types>;

struct Content {
    parts: Vec<Cid>
}

#[rocket::async_trait]
impl ContentAddressedStorage for Ipfs {
    type Error = anyhow::Error;
    async fn put(&self, content: &[u8], _codec: SupportedCodecs) -> Result<Cid, Self::Error> {
        let parts: Vec<Ipld> = content.chunks(KeplerParams::MAX_BLOCK_SIZE)
            .map(Vec::from)
            .map(Ipld::Bytes)
            .collect();
        
        let dag = Ipld::List(parts);
        let cid = self.put_dag(dag).await?;
        self.insert_pin(&cid, true).await?;
        Ok(cid)
    }
    async fn get(&self, address: &Cid) -> Result<Option<Vec<u8>>, Self::Error> {
        // TODO this api returns Result<Block, anyhow::Error>, with an err thrown for no block found
        // until this API changes (a breaking change), we will error here when no block found
        if let Ipld::List(parts) = self.get_dag(IpfsPath::new(PathRoot::Ipld(address.clone()))).await? {
        return Ok(Some(parts.into_iter()
            .try_fold(vec![], |mut acc, ipld| {
                if let Ipld::Bytes(mut part) = ipld {
                    acc.append(&mut part);
                    return Ok(acc)
                }
                Err(anyhow!("unexpected structure"))
            })?))
        } 
        Err(anyhow!("unexpected structure"))
    }

    async fn delete(&self, address: &Cid) -> Result<(), Self::Error> {
        // TODO: does not recursively remove blocks, some cleanup will need to happen.
        self.remove_pin(address, true).await?;
        self.remove_block(address.clone()).await?;
        Ok(())
    }
    async fn list(&self) -> Result<Vec<Cid>, Self::Error> {
        // return a list of all CIDs which are aliased/pinned
        self.list_pins(Some(PinMode::Recursive)).await.map_ok(|(cid, _pin_mode)| cid).try_collect().await
    }
}
