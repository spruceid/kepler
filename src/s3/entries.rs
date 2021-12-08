use super::{to_block, to_block_raw};
use crate::ipfs::{Block, Ipfs, KeplerParams};
use anyhow::Result;
use ipfs_embed::TempPin;
use libipld::{cbor::DagCborCodec, cid::Cid, store::StoreParams, DagCbor};
use std::{
    collections::BTreeMap,
    io::{self, Cursor, ErrorKind, Write},
    pin::Pin,
    task::{Context, Poll},
};

use rocket::tokio::io::{copy, AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};

pub struct IpfsReadStream {
    store: Ipfs,
    block: Cursor<Block>,
    index: usize,
    pub content: Vec<(Cid, u32)>,
}

impl IpfsReadStream {
    pub fn new(store: Ipfs, content: Vec<(Cid, u32)>) -> Result<Self> {
        let (cid0, _) = content.first().ok_or(anyhow!("Empty Content"))?;
        let block = Cursor::new(store.get(&cid0)?);
        Ok(Self {
            store,
            content,
            block,
            index: 0,
        })
    }
}

impl AsyncRead for IpfsReadStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<(), io::Error>> {
        let mut s = self.get_mut();
        let p = s.block.position();
        match Pin::new(&mut s.block).poll_read(cx, buf) {
            Poll::Ready(Ok(())) if p == s.block.position() => {
                tracing::debug!("read {} bytes from block {}", p, s.index);
                s.index += 1;
                // TODO probably not good to block here
                if let Some(block) = s
                    .content
                    .get(s.index)
                    .and_then(|(cid, _)| s.store.get(&cid).ok())
                {
                    tracing::debug!("loading block {} of {}", s.index + 1, s.content.len());
                    s.block = Cursor::new(block);
                    return Pin::new(&mut s).poll_read(cx, buf);
                }
                Poll::Ready(Ok(()))
            }
            e => e,
        }
    }
}

pub struct IpfsWriteStream<'a> {
    store: &'a Ipfs,
    pub content: Vec<(Cid, u32)>,
    pub pin: TempPin,
    buffer: Vec<u8>,
}

impl<'a> IpfsWriteStream<'a> {
    pub fn new(store: &'a Ipfs) -> anyhow::Result<Self> {
        Ok(Self {
            store,
            content: Default::default(),
            pin: store.create_temp_pin()?,
            buffer: Vec::new(),
        })
    }

    pub fn seal(mut self) -> anyhow::Result<(Cid, TempPin)> {
        self.flush_buffer_to_block()?;
        let block = to_block(&self.content)?;
        self.store.insert(&block)?;
        self.store.temp_pin(&self.pin, block.cid())?;
        Ok((*block.cid(), self.pin))
    }

    pub async fn write<R>(mut self, mut reader: R) -> anyhow::Result<(Cid, TempPin)>
    where
        R: AsyncRead + Unpin,
    {
        copy(&mut reader, &mut self).await?;
        self.flush().await?;
        self.seal()
    }

    fn flush_buffer_to_block(&mut self) -> Result<(), io::Error> {
        let len = KeplerParams::MAX_BLOCK_SIZE.min(self.buffer.len());

        if len > 0 {
            let (block_data, overflow) = self.buffer.split_at(len);
            let block =
                to_block_raw(&block_data).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            self.buffer = overflow.to_vec();
            tracing::debug!("flushing {} bytes to block {}", len, block.cid());
            self.store
                .insert(&block)
                .map_err(|e| io::Error::new(ErrorKind::Other, e))?;
            self.store
                .temp_pin(&self.pin, block.cid())
                .map_err(|e| io::Error::new(ErrorKind::Other, e))?;
            self.content.push((*block.cid(), len as u32));
            tracing::debug!(
                "block {} flushed {} bytes with {}",
                self.content.len(),
                len,
                block.cid()
            );
        }
        Ok(())
    }
}

impl<'a> AsyncWrite for IpfsWriteStream<'a> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let s = self.get_mut();

        let written = Write::write(&mut s.buffer, buf)?;

        if s.buffer.len() >= KeplerParams::MAX_BLOCK_SIZE {
            s.flush_buffer_to_block()?;
        }
        Poll::Ready(Ok(written))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }
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

#[tokio::test(flavor = "multi_thread")]
async fn write() -> Result<(), anyhow::Error> {
    crate::tracing_try_init();
    let tmp = tempdir::TempDir::new("test_streams")?;
    let data = vec![3u8; KeplerParams::MAX_BLOCK_SIZE * 3];

    let config = ipfs_embed::Config::new(&tmp.path(), ipfs_embed::generate_keypair());
    let ipfs = Ipfs::new(config).await?;

    let write = IpfsWriteStream::new(&ipfs)?;
    tracing::debug!("write");
    let (o, pins) = write.write(Cursor::new(data.clone())).await?;

    let content = ipfs.get(&o)?.decode::<DagCborCodec, Vec<(Cid, u32)>>()?;

    let mut read = IpfsReadStream::new(ipfs, content)?;

    let mut out = Vec::new();
    copy(&mut read, &mut out).await?;

    assert_eq!(out.len(), data.len());
    Ok(())
}
