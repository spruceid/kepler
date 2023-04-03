use anyhow::{Error, Result};
use kepler_lib::libipld::cid::{
    multihash::{Code, Error as MultihashError, Multihash},
    Cid,
};
use kepler_lib::resource::OrbitId;
use pin_project::pin_project;
use std::{collections::HashMap, error::Error as StdError};

pub mod either;
pub mod file_system;
mod indexes;
pub mod s3;
mod utils;

pub use indexes::KV;

#[async_trait]
pub trait StorageConfig<S> {
    type Error: StdError;
    async fn open(&self, orbit: &OrbitId) -> Result<Option<S>, Self::Error>;
    async fn create(&self, orbit: &OrbitId) -> Result<S, Self::Error>;
}

#[derive(thiserror::Error, Debug)]
pub enum VecReadError<E> {
    #[error(transparent)]
    Store(#[from] E),
    #[error(transparent)]
    Read(futures::io::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum KeyedWriteError<E> {
    #[error("Hash Mismatch")]
    IncorrectHash,
    #[error(transparent)]
    InvalidCode(MultihashError),
    #[error(transparent)]
    Store(#[from] E),
}

#[pin_project]
#[derive(Debug)]
pub struct Content<R> {
    size: u64,
    #[pin]
    content: R,
}

impl<R> Content<R> {
    pub fn new(size: u64, content: R) -> Self {
        Self { size, content }
    }

    pub fn len(&self) -> u64 {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn into_inner(self) -> (u64, R) {
        (self.size, self.content)
    }
}

impl<R> futures::io::AsyncRead for Content<R>
where
    R: futures::io::AsyncRead,
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.project();
        this.content.poll_read(cx, buf)
    }

    fn poll_read_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.project();
        this.content.poll_read_vectored(cx, bufs)
    }
}

/// A Store implementing content-addressed storage
/// Content is address by [Multihash][libipld::cid::multihash::Multihash] and represented as an
/// [AsyncRead][futures::io::AsyncRead]-implementing type.
#[async_trait]
pub trait ImmutableStore: Send + Sync {
    type Error: StdError + Send + Sync;
    type Readable: futures::io::AsyncRead + Send + Sync;
    async fn contains(&self, id: &Multihash) -> Result<bool, Self::Error>;
    async fn write(
        &self,
        data: impl futures::io::AsyncRead + Send,
        hash_type: Code,
    ) -> Result<Multihash, Self::Error>;
    async fn write_keyed(
        &self,
        data: impl futures::io::AsyncRead + Send,
        hash: &Multihash,
    ) -> Result<(), KeyedWriteError<Self::Error>>;
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error>;
    async fn read(&self, id: &Multihash) -> Result<Option<Content<Self::Readable>>, Self::Error>;
    async fn read_to_vec(
        &self,
        id: &Multihash,
    ) -> Result<Option<Vec<u8>>, VecReadError<Self::Error>>
    where
        Self::Readable: Send,
    {
        use futures::io::AsyncReadExt;
        let (l, r) = match self.read(id).await? {
            None => return Ok(None),
            Some(r) => r.into_inner(),
        };
        let mut v = Vec::with_capacity(l as usize);
        Box::pin(r)
            .read_to_end(&mut v)
            .await
            .map_err(VecReadError::Read)?;
        Ok(Some(v))
    }
    async fn total_size(&self) -> Result<u64, Self::Error>;
}

#[async_trait]
trait StoreSeek: ImmutableStore {
    type Seekable: futures::io::AsyncSeek;
    async fn seek(&self, id: &Cid) -> Result<Option<Self::Seekable>, Self::Error>;
}

#[async_trait]
impl<S> ImmutableStore for Box<S>
where
    S: ImmutableStore + Send + Sync,
{
    type Error = S::Error;
    type Readable = S::Readable;
    async fn contains(&self, id: &Multihash) -> Result<bool, Self::Error> {
        self.contains(id).await
    }
    async fn write(
        &self,
        data: impl futures::io::AsyncRead + Send,
        hash_type: Code,
    ) -> Result<Multihash, Self::Error> {
        self.write(data, hash_type).await
    }
    async fn write_keyed(
        &self,
        data: impl futures::io::AsyncRead + Send,
        hash: &Multihash,
    ) -> Result<(), KeyedWriteError<Self::Error>> {
        self.write_keyed(data, hash).await
    }
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error> {
        self.remove(id).await
    }
    async fn read(&self, id: &Multihash) -> Result<Option<Content<Self::Readable>>, Self::Error> {
        self.read(id).await
    }
    async fn read_to_vec(
        &self,
        id: &Multihash,
    ) -> Result<Option<Vec<u8>>, VecReadError<Self::Error>>
    where
        Self::Readable: Send,
    {
        self.read_to_vec(id).await
    }
    async fn total_size(&self) -> Result<u64, Self::Error> {
        self.total_size().await
    }
}

#[async_trait]
trait IdempotentHeightGroup {
    // write a height value for a Cid
    // should error if given value already exists
    // if successful, marks a Cid as 'fresh'
    async fn see(&self, id: impl IntoIterator<Item = (&Cid, &u64)>) -> Result<(), Error>;
    // mark a Cid as no longer 'fresh'
    async fn stale(&self, id: impl IntoIterator<Item = &Cid>) -> Result<(), Error>;
    // return 'fresh' Cids and their heights
    async fn fresh(&self) -> Result<HashMap<Cid, u64>, Error>;
    // return the heights of any Cids
    async fn height<'a>(
        &self,
        id: impl IntoIterator<Item = &'a Cid>,
    ) -> Result<HashMap<&'a Cid, u64>, Error>;
}
