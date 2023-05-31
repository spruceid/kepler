pub mod file_system;
pub mod s3;

use crate::orbit::ProviderUtils;
use kepler_core::storage::either::{Either, EitherError};
use kepler_lib::resource::OrbitId;
use libp2p::identity::Keypair;

#[async_trait]
impl<A, B> ProviderUtils for Either<A, B>
where
    A: ProviderUtils + Send + Sync,
    B: ProviderUtils + Send + Sync,
{
    type Error = EitherError<A::Error, B::Error>;
    async fn exists(&self, orbit: &OrbitId) -> Result<bool, Self::Error> {
        match self {
            Either::A(a) => a.exists(orbit).await.map_err(EitherError::A),
            Either::B(b) => b.exists(orbit).await.map_err(EitherError::B),
        }
    }
    async fn relay_key_pair(&self) -> Result<Keypair, Self::Error> {
        match self {
            Either::A(a) => a.relay_key_pair().await.map_err(EitherError::A),
            Either::B(b) => b.relay_key_pair().await.map_err(EitherError::B),
        }
    }
    async fn key_pair(&self, orbit: &OrbitId) -> Result<Option<Keypair>, Self::Error> {
        match self {
            Either::A(a) => a.key_pair(orbit).await.map_err(EitherError::A),
            Either::B(b) => b.key_pair(orbit).await.map_err(EitherError::B),
        }
    }
    async fn setup_orbit(&self, orbit: &OrbitId, key: &Keypair) -> Result<(), Self::Error> {
        match self {
            Either::A(a) => a.setup_orbit(orbit, key).await.map_err(EitherError::A),
            Either::B(b) => b.setup_orbit(orbit, key).await.map_err(EitherError::B),
        }
    }
}
