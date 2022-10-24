use super::ImmutableStore;
use core::pin::Pin;
use futures::{
    io::{AsyncRead, Error},
    task::{Context, Poll},
};
use kepler_lib::libipld::cid::multihash::Multihash;

#[derive(Debug)]
pub enum EitherStore<L, R> {
    Left(L),
    Right(R),
}

#[derive(Debug)]
pub enum AsyncReadEither<L, R>
where
    L: ImmutableStore,
    R: ImmutableStore,
{
    Left(L::Readable),
    Right(R::Readable),
}

impl<L, R> AsyncRead for AsyncReadEither<L, R>
where
    L: ImmutableStore,
    R: ImmutableStore,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, Error>> {
        match self {
            Self::L(l) => l.poll_read(cx, buf),
            Self::R(r) => r.poll_read(cx, buf),
        }
    }
}

#[derive(thiserror::Error)]
pub enum EitherStoreError<L, R>
where
    L: ImmutableStore,
    R: ImmutableStore,
{
    #[error(transparent)]
    Left(L::Error),
    #[error(transparent)]
    Right(R::Error),
}

#[async_trait]
impl<L, R> ImmutableStore for EitherStore<L, R>
where
    L: ImmutableStore,
    R: ImmutableStore,
{
    type Readable = AsyncReadEither<L, R>;
    type Error = EitherStoreError<L, R>;
    async fn contains(&self, id: &Multihash) -> Result<bool, Self::Error> {
        match self {
            Self::Left(l) => l.contains(id).await.map_err(Self::Error::Left),
            Self::Right(r) => r.contains(id).await.map_err(Self::Error::Right),
        }
    }
    async fn write(&self, data: impl futures::io::AsyncRead) -> Result<Multihash, Self::Error> {
        match self {
            Self::Left(l) => l.write(data).await.map_err(Self::Error::Left),
            Self::Right(r) => r.write(data).await.map_err(Self::Error::Right),
        }
    }
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error> {
        match self {
            Self::Left(l) => l.remove(id).await.map_err(Self::Error::Left),
            Self::Right(r) => r.remove(id).await.map_err(Self::Error::Right),
        }
    }
    async fn read(&self, id: &Multihash) -> Result<Option<Self::Readable>, Self::Error> {
        match self {
            Self::Left(l) => l.read(id).await.map_err(Self::Error::Left),
            Self::Right(r) => r.read(id).await.map_err(Self::Error::Right),
        }
    }
}
