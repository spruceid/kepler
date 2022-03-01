use super::{to_block, to_block_raw};
use crate::ipfs::{Block, Ipfs, KeplerParams};
use anyhow::Result;
use libipld::{cid::Cid, store::StoreParams, DagCbor};
use libp2p::futures::stream::BoxStream;
use std::{
    collections::BTreeMap,
    io::{self, Cursor, ErrorKind, Write},
};

use rocket::futures::{StreamExt, TryStreamExt};
use rocket::tokio::io::AsyncRead;
use tokio_stream::iter;
use tokio_util::io::{ReaderStream, StreamReader};

pub type ObjectReader =
    StreamReader<BoxStream<'static, Result<Cursor<Block>, io::Error>>, Cursor<Block>>;

pub fn read_from_store(ipfs: Ipfs, content: Vec<(Cid, u32)>) -> ObjectReader {
    let chunk_stream = Box::pin(
        iter(content)
            .then(move |(cid, _)| get_block(ipfs.clone(), cid))
            .map_ok(Cursor::new)
            .map_err(|ipfs_err| io::Error::new(ErrorKind::Other, ipfs_err)),
    );
    StreamReader::new(chunk_stream)
}

async fn get_block(ipfs: Ipfs, cid: Cid) -> Result<Block, ipfs::Error> {
    ipfs.get_block(&cid).await
}

pub async fn write_to_store<R>(store: &Ipfs, source: R) -> anyhow::Result<Cid>
where
    R: AsyncRead + Unpin,
{
    let mut reader = ReaderStream::new(source);
    let mut buffer: Vec<u8> = Vec::new();
    let mut content: Vec<(Cid, u32)> = Vec::new();
    while let Some(chunk) = reader.next().await.transpose()? {
        buffer.write_all(&chunk)?;
        while buffer.len() >= KeplerParams::MAX_BLOCK_SIZE {
            flush_buffer_to_block(store, &mut buffer, &mut content).await?;
        }
    }
    while !buffer.is_empty() {
        flush_buffer_to_block(store, &mut buffer, &mut content).await?;
    }
    let block = to_block(&content)?;
    let cid = store.put_block(block).await?;
    Ok(cid)
}

async fn flush_buffer_to_block(
    store: &Ipfs,
    buffer: &mut Vec<u8>,
    content: &mut Vec<(Cid, u32)>,
) -> Result<(), io::Error> {
    let len = KeplerParams::MAX_BLOCK_SIZE.min(buffer.len());
    if len > 0 {
        let (block_data, overflow) = buffer.split_at(len);
        let block =
            to_block_raw(&block_data).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
        *buffer = overflow.to_vec();
        tracing::debug!("flushing {} bytes to block {}", len, block.cid());
        let cid = store
            .put_block(block)
            .await
            .map_err(|e| io::Error::new(ErrorKind::Other, e))?;
        tracing::debug!("block {} flushed {} bytes with {}", content.len(), len, cid);
        content.push((cid, len as u32));
    }
    Ok(())
}

#[derive(DagCbor, PartialEq, Debug, Clone)]
pub struct Object {
    pub key: Vec<u8>,
    pub value: Cid,
    pub metadata: BTreeMap<String, String>,
}

impl Object {
    pub fn new(
        key: Vec<u8>,
        value: Cid,
        metadata: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        Self {
            key,
            value,
            metadata: metadata.into_iter().collect(),
        }
    }

    pub fn to_block(&self) -> Result<Block> {
        to_block(self)
    }
}

pub struct ObjectBuilder {
    pub key: Vec<u8>,
    pub metadata: BTreeMap<String, String>,
}

impl ObjectBuilder {
    pub fn new(key: Vec<u8>, metadata: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            key,
            metadata: metadata.into_iter().collect(),
        }
    }

    pub fn add_content(self, value: Cid) -> Object {
        Object::new(self.key, value, self.metadata)
    }
}

#[cfg(test)]
mod test {
    use ipfs::IpfsOptions;

    use super::*;
    use crate::s3::DagCborCodec;

    #[tokio::test(flavor = "multi_thread")]
    async fn write() -> Result<(), anyhow::Error> {
        crate::tracing_try_init();
        let tmp = tempdir::TempDir::new("test_streams")?;
        let data = vec![3u8; KeplerParams::MAX_BLOCK_SIZE * 3];

        let mut config = IpfsOptions::inmemory_with_generated_keys();
        config.ipfs_path = tmp.path().into();

        let (ipfs, task) = config.create_uninitialised_ipfs()?.start().await?;
        let _join_handle = tokio::spawn(task);

        let o = write_to_store(&ipfs, Cursor::new(data.clone())).await?;

        let content = ipfs
            .get_block(&o)
            .await?
            .decode::<DagCborCodec, Vec<(Cid, u32)>>()?;

        let mut read = read_from_store(ipfs, content);

        let mut out = Vec::new();
        tokio::io::copy(&mut read, &mut out).await?;

        assert_eq!(out.len(), data.len());
        Ok(())
    }
}
