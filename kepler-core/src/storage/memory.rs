use crate::storage::ImmutableStaging;
use sea_orm_migration::async_trait::async_trait;

pub struct MemoryStaging;

#[async_trait]
impl ImmutableStaging for MemoryStaging {
    type Writable = Vec<u8>;
    type Error = std::io::Error;
    async fn get_staging_buffer(&self) -> Result<Self::Writable, Self::Error> {
        Ok(Vec::new())
    }
}
