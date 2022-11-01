use crate::{
    orbit::ProviderUtils,
    storage::{ImmutableStore, StorageConfig},
};
use core::pin::Pin;
use futures::{
    io::{AsyncRead, Error},
    task::{Context, Poll},
};
use kepler_lib::{
    libipld::cid::{multihash::Multihash, Cid},
    resource::OrbitId,
};
use libp2p::identity::ed25519::Keypair as Ed25519Keypair;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum Either<A, B> {
    A(A),
    B(B),
}

#[derive(Debug, Clone)]
pub enum AsyncReadEither<A, B>
where
    A: ImmutableStore,
    B: ImmutableStore,
{
    A(A::Readable),
    B(B::Readable),
}

impl<A, B> AsyncRead for AsyncReadEither<A, B>
where
    A: ImmutableStore,
    B: ImmutableStore,
{
    #[inline]
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, Error>> {
        // it actually seems like this is only possible with unsafe :(
        // TODO use pin-project crate
        unsafe {
            match self.get_unchecked_mut() {
                Self::A(l) => AsyncRead::poll_read(Pin::new_unchecked(l), cx, buf),
                Self::B(r) => AsyncRead::poll_read(Pin::new_unchecked(r), cx, buf),
            }
        }
    }
    #[inline]
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
    ) -> Poll<Result<usize, Error>> {
        unsafe {
            match self.get_unchecked_mut() {
                Self::A(l) => AsyncRead::poll_read_vectored(Pin::new_unchecked(l), cx, bufs),
                Self::B(r) => AsyncRead::poll_read_vectored(Pin::new_unchecked(r), cx, bufs),
            }
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum EitherError<A, B> {
    #[error(transparent)]
    A(A),
    #[error(transparent)]
    B(B),
}

#[async_trait]
impl<A, B> ImmutableStore for Either<A, B>
where
    A: ImmutableStore,
    B: ImmutableStore,
{
    type Readable = AsyncReadEither<A, B>;
    type Error = EitherError<A::Error, B::Error>;
    async fn contains(&self, id: &Multihash) -> Result<bool, Self::Error> {
        match self {
            Self::A(l) => l.contains(id).await.map_err(Self::Error::A),
            Self::B(r) => r.contains(id).await.map_err(Self::Error::B),
        }
    }
    async fn write(
        &self,
        data: impl futures::io::AsyncRead + Send,
    ) -> Result<Multihash, Self::Error> {
        match self {
            Self::A(l) => l.write(data).await.map_err(Self::Error::A),
            Self::B(r) => r.write(data).await.map_err(Self::Error::B),
        }
    }
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error> {
        match self {
            Self::A(l) => l.remove(id).await.map_err(Self::Error::A),
            Self::B(r) => r.remove(id).await.map_err(Self::Error::B),
        }
    }
    async fn read(&self, id: &Multihash) -> Result<Option<Self::Readable>, Self::Error> {
        match self {
            Self::A(l) => l
                .read(id)
                .await
                .map(|o| o.map(Self::Readable::A))
                .map_err(Self::Error::A),
            Self::B(r) => r
                .read(id)
                .await
                .map(|o| o.map(Self::Readable::B))
                .map_err(Self::Error::B),
        }
    }
}

#[async_trait]
impl<A, B, SA, SB> StorageConfig<Either<SA, SB>> for Either<A, B>
where
    A: StorageConfig<SA> + Sync,
    B: StorageConfig<SB> + Sync,
{
    type Error = EitherError<A::Error, B::Error>;
    async fn open(&self, orbit: &OrbitId) -> Result<Option<Either<SA, SB>>, Self::Error> {
        match self {
            Self::A(a) => a
                .open(orbit)
                .await
                .map(|o| o.map(Either::A))
                .map_err(Self::Error::A),
            Self::B(b) => b
                .open(orbit)
                .await
                .map(|o| o.map(Either::B))
                .map_err(Self::Error::B),
        }
    }
    async fn create(&self, orbit: &OrbitId) -> Result<Either<SA, SB>, Self::Error> {
        match self {
            Self::A(a) => a.create(orbit).await.map(Either::A).map_err(Self::Error::A),
            Self::B(b) => b.create(orbit).await.map(Either::B).map_err(Self::Error::B),
        }
    }
}

#[async_trait]
impl<A, B> ProviderUtils for Either<A, B>
where
    A: ProviderUtils + Sync,
    B: ProviderUtils + Sync,
{
    type Error = EitherError<A::Error, B::Error>;
    async fn exists(&self, orbit: &OrbitId) -> Result<bool, Self::Error> {
        match self {
            Self::A(a) => a.exists(orbit).await.map_err(Self::Error::A),
            Self::B(b) => b.exists(orbit).await.map_err(Self::Error::B),
        }
    }
    async fn relay_key_pair(&self) -> Result<Ed25519Keypair, Self::Error> {
        match self {
            Self::A(a) => a.relay_key_pair().await.map_err(Self::Error::A),
            Self::B(b) => b.relay_key_pair().await.map_err(Self::Error::B),
        }
    }
    async fn key_pair(&self, orbit: &OrbitId) -> Result<Option<Ed25519Keypair>, Self::Error> {
        match self {
            Self::A(a) => a.key_pair(orbit).await.map_err(Self::Error::A),
            Self::B(b) => b.key_pair(orbit).await.map_err(Self::Error::B),
        }
    }
    async fn setup_orbit(&self, orbit: &OrbitId, key: &Ed25519Keypair) -> Result<(), Self::Error> {
        match self {
            Self::A(a) => a.setup_orbit(orbit, key).await.map_err(Self::Error::A),
            Self::B(b) => b.setup_orbit(orbit, key).await.map_err(Self::Error::B),
        }
    }
}
