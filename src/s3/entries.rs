use super::{to_block, to_block_raw};
use crate::ipfs::{Block, Ipfs, KeplerParams};
use anyhow::Result;
use ipfs_embed::TempPin;
use libipld::{cid::Cid, store::StoreParams, DagCbor};
use std::{
    collections::BTreeMap,
    io::{self, ErrorKind, Cursor, Write},
    pin::Pin,
    task::{Context, Poll},
};


use rocket::{
    tokio::io::{AsyncWrite, AsyncRead, copy, BufWriter, AsyncWriteExt, BufReader, ReadBuf},
};

pub struct IpfsReadStream {
    store: Ipfs,
    block: Cursor<Block>,
    index: usize,
    pub content: Vec<(Cid, u32)>,
}

impl IpfsReadStream {
    pub fn new(store: Ipfs, mut content: Vec<(Cid, u32)>) -> Result<Self> {
        let (cid0, _) = content.pop().ok_or(anyhow!("Enpty Content"))?;
        let block = Cursor::new(store.get(&cid0)?);
        Ok(Self { store, content, block, index: 0 })
    }
}

impl AsyncRead for IpfsReadStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>
    ) -> Poll<Result<(), io::Error>> {
        let mut s = self.get_mut();
        let p = s.block.position();
        match Pin::new(&mut s.block).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => if p == s.block.position() {
                match s.content.get(s.index + 1).and_then(|(cid, _)| 
                    s.store.get(&cid).ok()
                ) {
                    Some(block) => {
                        s.index += 1;
                        s.block = Cursor::new(block);
                        tracing::debug!("loading block {}", s.index);
                        Pin::new(&mut s).poll_read(cx, buf)
                    },
                    None => Poll::Ready(Ok(()))
                }
            } else {
                Poll::Ready(Ok(()))
            },
            e => e
        }
    }
}

pub struct IpfsWriteStream<'a> {
    store: &'a Ipfs,
    pub content: Vec<(Cid, u32)>,
    pub pin: TempPin,
    buffer: [u8; KeplerParams::MAX_BLOCK_SIZE],
    offset: usize
}

impl<'a> IpfsWriteStream<'a> {
    pub fn new(store: &'a Ipfs) -> anyhow::Result<Self> {
        Ok(Self {
            store,
            content: Default::default(),
            pin: store.create_temp_pin()?,
            buffer: [0u8; KeplerParams::MAX_BLOCK_SIZE],
            offset: 0
        })
    }

    pub fn seal(self) -> anyhow::Result<(Cid, TempPin)> {
        let block = to_block(&self.content)?;
        self.store.insert(&block)?;
        self.store.temp_pin(&self.pin, block.cid())?;
        Ok((*block.cid(), self.pin))
    }

    pub async fn write<R>(self, mut reader: R) -> anyhow::Result<(Cid, TempPin)> where R: AsyncRead + Unpin {
        let mut bw = BufWriter::with_capacity(KeplerParams::MAX_BLOCK_SIZE, self);
        copy(&mut reader, &mut bw).await?;
        bw.flush().await?;
        bw.into_inner().seal()
    }
}

impl<'a> AsyncWrite for IpfsWriteStream<'a> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        if buf.len() == 0 {
            return Poll::Ready(Ok(0));
        };

        let mut s = self.get_mut();
        let mut remaining = &mut s.buffer[s.offset..];

        let to_write = buf.len().min(remaining.len());
        tracing::debug!("writing {} bytes at {} to block {}", to_write, s.offset, s.content.len());

        remaining.write_all(&buf[..to_write])?;
        s.offset = KeplerParams::MAX_BLOCK_SIZE - remaining.len();

        if s.offset == KeplerParams::MAX_BLOCK_SIZE {
            let block = to_block_raw(&s.buffer.to_vec()).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            s.store
                .insert(&block)
                .map_err(|e| io::Error::new(ErrorKind::Other, e))?;
            s.store
                .temp_pin(&s.pin, block.cid())
                .map_err(|e| io::Error::new(ErrorKind::Other, e))?;

            s.content.push((*block.cid(), s.offset as u32));
            s.offset = 0;
            Pin::new(&mut s).poll_write(cx, &buf[to_write..])
        } else {
            Poll::Ready(Ok(to_write))
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        if self.offset > 0 {
            let block = to_block_raw(&self.buffer[..self.offset].to_vec()).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            self.store
                .insert(&block)
                .map_err(|e| io::Error::new(ErrorKind::Other, e))?;
            self.store
                .temp_pin(&self.pin, block.cid())
                .map_err(|e| io::Error::new(ErrorKind::Other, e))?;

            let mut s = self.get_mut();
            s.content.push((*block.cid(), s.offset as u32));
            s.offset = 0;
        }
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
    
    let config = ipfs_embed::Config::new(&tmp.path(), ipfs_embed::generate_keypair());
    let ipfs = Ipfs::new(config).await?;

    let streamwrite = IpfsWriteStream::new(&ipfs)?;
    tracing::debug!("write");
    streamwrite.write(Cursor::new([255u8; 1111111111])).await?;
    Ok(())
}
