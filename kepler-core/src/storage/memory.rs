use crate::storage::{ImmutableStaging, StorageConfig};
use kepler_lib::resource::OrbitId;
use sea_orm_migration::async_trait::async_trait;

#[derive(Default, Debug, Clone, Hash, PartialEq, Eq)]
pub struct MemoryStaging;

#[async_trait]
impl ImmutableStaging for MemoryStaging {
    type Writable = Vec<u8>;
    type Error = std::io::Error;
    async fn get_staging_buffer(&self, _: &OrbitId) -> Result<Self::Writable, Self::Error> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl StorageConfig<MemoryStaging> for MemoryStaging {
    type Error = std::convert::Infallible;
    async fn open(&self) -> Result<MemoryStaging, Self::Error> {
        Ok(Self)
    }
}
