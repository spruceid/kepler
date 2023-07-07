use crate::{hash::Hash, storage::*};
use futures::future::Either as AsyncEither;
use kepler_lib::resource::OrbitId;
use sea_orm_migration::async_trait::async_trait;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum Either<A, B> {
    A(A),
    B(B),
}

#[derive(thiserror::Error, Debug)]
pub enum EitherError<A, B> {
    #[error(transparent)]
    A(A),
    #[error(transparent)]
    B(B),
}

#[async_trait]
impl<A, B> ImmutableReadStore for Either<A, B>
where
    A: ImmutableReadStore,
    B: ImmutableReadStore,
{
    type Readable = AsyncEither<A::Readable, B::Readable>;
    type Error = EitherError<A::Error, B::Error>;
    async fn contains(&self, orbit: &OrbitId, id: &Hash) -> Result<bool, Self::Error> {
        match self {
            Self::A(l) => l.contains(orbit, id).await.map_err(Self::Error::A),
            Self::B(r) => r.contains(orbit, id).await.map_err(Self::Error::B),
        }
    }
    async fn read(
        &self,
        orbit: &OrbitId,
        id: &Hash,
    ) -> Result<Option<Content<Self::Readable>>, Self::Error> {
        match self {
            Self::A(l) => l
                .read(orbit, id)
                .await
                .map(|o| {
                    o.map(|c| {
                        let (l, r) = c.into_inner();
                        Content::new(l, Self::Readable::Left(r))
                    })
                })
                .map_err(Self::Error::A),
            Self::B(r) => r
                .read(orbit, id)
                .await
                .map(|o| {
                    o.map(|c| {
                        let (l, r) = c.into_inner();
                        Content::new(l, Self::Readable::Right(r))
                    })
                })
                .map_err(Self::Error::B),
        }
    }
}

#[async_trait]
impl<A, B> ImmutableStaging for Either<A, B>
where
    A: ImmutableStaging,
    B: ImmutableStaging,
{
    type Writable = AsyncEither<A::Writable, B::Writable>;
    type Error = EitherError<A::Error, B::Error>;
    async fn get_staging_buffer(&self, orbit: &OrbitId) -> Result<Self::Writable, Self::Error> {
        match self {
            Self::A(l) => l
                .get_staging_buffer(orbit)
                .await
                .map(AsyncEither::Left)
                .map_err(Self::Error::A),
            Self::B(r) => r
                .get_staging_buffer(orbit)
                .await
                .map(AsyncEither::Right)
                .map_err(Self::Error::B),
        }
    }
}

#[async_trait]
impl<A, B, S> ImmutableWriteStore<S> for Either<A, B>
where
    A: ImmutableWriteStore<S>,
    B: ImmutableWriteStore<S>,
    S: ImmutableStaging,
    S::Writable: 'static,
{
    type Error = EitherError<A::Error, B::Error>;
    async fn persist(
        &self,
        orbit: &OrbitId,
        staged: HashBuffer<S::Writable>,
    ) -> Result<Hash, Self::Error> {
        match self {
            Self::A(a) => a.persist(orbit, staged).await.map_err(Self::Error::A),
            Self::B(b) => b.persist(orbit, staged).await.map_err(Self::Error::B),
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
    async fn open(&self) -> Result<Either<SA, SB>, Self::Error> {
        match self {
            Self::A(a) => a.open().await.map(Either::A).map_err(Self::Error::A),
            Self::B(b) => b.open().await.map(Either::B).map_err(Self::Error::B),
        }
    }
}

#[async_trait]
impl<A, B> StorageSetup for Either<A, B>
where
    A: StorageSetup + Sync,
    B: StorageSetup + Sync,
{
    type Error = EitherError<A::Error, B::Error>;
    async fn create(&self, orbit: &OrbitId) -> Result<(), Self::Error> {
        match self {
            Self::A(a) => a.create(orbit).await.map_err(Self::Error::A),
            Self::B(b) => b.create(orbit).await.map_err(Self::Error::B),
        }
    }
}

#[async_trait]
impl<A, B> ImmutableDeleteStore for Either<A, B>
where
    A: ImmutableDeleteStore,
    B: ImmutableDeleteStore,
{
    type Error = EitherError<A::Error, B::Error>;
    async fn remove(&self, orbit: &OrbitId, id: &Hash) -> Result<Option<()>, Self::Error> {
        match self {
            Self::A(l) => l.remove(orbit, id).await.map_err(Self::Error::A),
            Self::B(r) => r.remove(orbit, id).await.map_err(Self::Error::B),
        }
    }
}

#[async_trait]
impl<A, B> StoreSize for Either<A, B>
where
    A: StoreSize,
    B: StoreSize,
{
    type Error = EitherError<A::Error, B::Error>;
    async fn total_size(&self, orbit: &OrbitId) -> Result<Option<u64>, Self::Error> {
        match self {
            Either::A(a) => a.total_size(orbit).await.map_err(EitherError::A),
            Either::B(b) => b.total_size(orbit).await.map_err(EitherError::B),
        }
    }
}
