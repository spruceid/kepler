use super::{to_block, to_block_raw};
use crate::ipfs::{Block, Ipfs, KeplerParams};
use anyhow::Result;
use ipfs_embed::TempPin;
use libipld::{cid::Cid, store::StoreParams, DagCbor};
use std::{
    collections::BTreeMap,
    io::{self, ErrorKind},
    pin::Pin,
    task::{Context, Poll},
};

use rocket::tokio::io::{AsyncWrite, AsyncRead, copy, BufWriter, AsyncWriteExt};

pub struct IpfsWriteStream<'a> {
    store: &'a Ipfs,
    pub content: Vec<(Cid, u32)>,
    pub pin: TempPin,
}

impl<'a> IpfsWriteStream<'a> {
    pub fn new(store: &'a Ipfs) -> anyhow::Result<Self> {
        Ok(Self {
            store,
            content: Default::default(),
            pin: store.create_temp_pin()?,
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
        let t: &[u8] = &buf[..KeplerParams::MAX_BLOCK_SIZE.min(buf.len())];
        if t.len() == 0 {
            return Poll::Ready(Ok(0));
        };
        tracing::debug!("writing bytes: {}", t.len());
        let block = to_block_raw(&t.to_vec()).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
        self.store
            .insert(&block)
            .map_err(|e| io::Error::new(ErrorKind::Other, e))?;
        self.store
            .temp_pin(&self.pin, block.cid())
            .map_err(|e| io::Error::new(ErrorKind::Other, e))?;

        let c: &mut Vec<(Cid, u32)> = &mut self.get_mut().content;
        c.push((*block.cid(), t.len() as u32));

        Poll::Ready(Ok(t.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
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
