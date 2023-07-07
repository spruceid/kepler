use aws_sdk_s3::{
    error::{
        GetObjectAttributesError, GetObjectAttributesErrorKind, GetObjectError, GetObjectErrorKind,
        HeadObjectError, HeadObjectErrorKind,
    },
    types::{ByteStream, SdkError},
    Client, // Config,
    Error as S3Error,
};
use aws_smithy_http::{byte_stream::Error as ByteStreamError, endpoint::Endpoint};
use aws_types::sdk_config::SdkConfig;
use futures::{
    future::Either as AsyncEither,
    stream::{IntoAsyncRead, MapErr, TryStreamExt},
};
use kepler_core::{hash::Hash, storage::*};
use kepler_lib::resource::OrbitId;
use rocket::{async_trait, http::hyper::Uri};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use std::{collections::HashMap, io::Error as IoError, ops::AddAssign};

use super::{file_system, size::OrbitSizes};

async fn aws_config() -> SdkConfig {
    aws_config::from_env().load().await
}

#[derive(Debug, Clone)]
pub struct S3BlockStore {
    pub client: Client,
    pub bucket: String,
    sizes: OrbitSizes,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct S3BlockConfig {
    pub bucket: String,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub endpoint: Option<Uri>,
}

#[async_trait]
impl StorageConfig<S3BlockStore> for S3BlockConfig {
    type Error = S3Error;
    async fn open(&self) -> Result<S3BlockStore, Self::Error> {
        S3BlockStore::new_(self).await
    }
}

#[async_trait]
impl StorageSetup for S3BlockStore {
    type Error = std::convert::Infallible;
    async fn create(&self, orbit: &OrbitId) -> Result<(), Self::Error> {
        self.sizes.init_size(orbit.clone()).await;
        Ok(())
    }
}

async fn new_client(config: &S3BlockConfig) -> Client {
    let general_config = aws_config().await;
    let sdk_config = aws_sdk_s3::config::Builder::from(&general_config);
    let sdk_config = match &config.endpoint {
        Some(e) => sdk_config.endpoint_resolver(Endpoint::immutable(e.clone())),
        None => sdk_config,
    };
    let sdk_config = sdk_config.build();
    Client::from_conf(sdk_config)
}

impl S3BlockStore {
    async fn new_(config: &S3BlockConfig) -> Result<Self, S3Error> {
        let client = new_client(config).await;
        let sizes = client
            .list_objects_v2()
            .bucket(&config.bucket)
            .into_paginator()
            .send()
            // get the sum of all objects in each page
            .try_fold(HashMap::new(), |mut acc, page| async move {
                // get the sum of all objects per orbit in this particular page
                for (orbit, obj_size) in page.contents.into_iter().flatten().filter_map(|content| {
                    content.key().and_then(|key| {
                        let (o, _) = key.rsplit_once('/')?;
                        let orbit: OrbitId = o.parse().ok()?;
                        if content.size() > 0 {
                            Some((orbit, content.size() as u64))
                        } else {
                            None
                        }
                    })
                }) {
                    acc.entry(orbit).or_insert(0).add_assign(obj_size);
                }
                Ok(acc)
            })
            .await?
            .into();
        Ok(S3BlockStore {
            client,
            bucket: config.bucket.clone(),
            sizes,
        })
    }

    fn key(&self, orbit: &OrbitId, id: &Hash) -> String {
        format!(
            "{}/{}",
            orbit,
            base64::encode_config(id.as_ref(), base64::URL_SAFE)
        )
    }

    async fn increment_size(&self, orbit: &OrbitId, size: u64) {
        self.sizes.increment_size(orbit, size).await;
    }
    async fn decrement_size(&self, orbit: &OrbitId, size: u64) {
        self.sizes.decrement_size(orbit, size).await;
    }
}

pub fn convert(e: ByteStreamError) -> IoError {
    e.into()
}

#[derive(thiserror::Error, Debug)]
pub enum S3StoreError {
    #[error(transparent)]
    S3(#[from] S3Error),
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    Bytestream(#[from] ByteStreamError),
    #[error(transparent)]
    Length(#[from] std::num::TryFromIntError),
}

#[async_trait]
impl ImmutableReadStore for S3BlockStore {
    type Error = S3StoreError;
    type Readable = IntoAsyncRead<MapErr<ByteStream, fn(ByteStreamError) -> IoError>>;
    async fn contains(&self, orbit: &OrbitId, id: &Hash) -> Result<bool, Self::Error> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(self.key(orbit, id))
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(SdkError::ServiceError {
                err:
                    HeadObjectError {
                        kind: HeadObjectErrorKind::NotFound(_),
                        ..
                    },
                ..
            }) => Ok(false),
            Err(e) => Err(S3Error::from(e).into()),
        }
    }

    async fn read(
        &self,
        orbit: &OrbitId,
        id: &Hash,
    ) -> Result<Option<Content<Self::Readable>>, Self::Error> {
        let res = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(self.key(orbit, id))
            .send()
            .await;
        match res {
            Ok(o) => Ok(Some(Content::new(
                o.content_length().try_into()?,
                o.body
                    .map_err(convert as fn(ByteStreamError) -> IoError)
                    .into_async_read(),
            ))),
            Err(SdkError::ServiceError {
                err:
                    GetObjectError {
                        kind: GetObjectErrorKind::NoSuchKey(_),
                        ..
                    },
                ..
            }) => Ok(None),
            Err(e) => Err(S3Error::from(e).into()),
        }
    }
}

#[async_trait]
impl ImmutableWriteStore<memory::MemoryStaging> for S3BlockStore {
    type Error = S3StoreError;
    async fn persist(
        &self,
        orbit: &OrbitId,
        staged: HashBuffer<<memory::MemoryStaging as ImmutableStaging>::Writable>,
    ) -> Result<Hash, Self::Error> {
        let (mut h, f) = staged.into_inner();
        let hash = h.finalize();

        if !self.contains(orbit, &hash).await? {
            let size = f.len() as u64;
            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(self.key(orbit, &hash))
                .body(ByteStream::from(f))
                .send()
                .await
                .map_err(S3Error::from)?;
            self.increment_size(orbit, size).await;
        }
        Ok(hash)
    }
}

#[async_trait]
impl ImmutableWriteStore<file_system::TempFileSystemStage> for S3BlockStore {
    type Error = S3StoreError;
    async fn persist(
        &self,
        orbit: &OrbitId,
        staged: HashBuffer<<file_system::TempFileSystemStage as ImmutableStaging>::Writable>,
    ) -> Result<Hash, Self::Error> {
        let (mut h, f) = staged.into_inner();
        let hash = h.finalize();

        if !self.contains(orbit, &hash).await? {
            let size = f.size().await?;
            let (_file, path) = f.into_inner();

            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(self.key(orbit, &hash))
                .body(ByteStream::from_path(&path).await?)
                .send()
                .await
                .map_err(S3Error::from)?;
            self.increment_size(orbit, size).await;
        }
        Ok(hash)
    }
}

#[async_trait]
impl ImmutableWriteStore<either::Either<file_system::TempFileSystemStage, memory::MemoryStaging>>
    for S3BlockStore
{
    type Error = S3StoreError;
    async fn persist(
        &self,
        orbit: &OrbitId,
        staged: HashBuffer<<either::Either<file_system::TempFileSystemStage, memory::MemoryStaging> as ImmutableStaging>::Writable>,
    ) -> Result<Hash, Self::Error> {
        let (mut h, f) = staged.into_inner();
        let hash = h.finalize();

        if !self.contains(orbit, &hash).await? {
            match f {
                AsyncEither::Left(t_file) => {
                    let size = t_file.size().await?;
                    let (_file, path) = t_file.into_inner();
                    self.client
                        .put_object()
                        .bucket(&self.bucket)
                        .key(self.key(orbit, &hash))
                        .body(ByteStream::from_path(&path).await?)
                        .send()
                        .await
                        .map_err(S3Error::from)?;
                    self.increment_size(orbit, size).await;
                }
                AsyncEither::Right(b) => {
                    let size = b.len() as u64;
                    self.client
                        .put_object()
                        .bucket(&self.bucket)
                        .key(self.key(orbit, &hash))
                        .body(ByteStream::from(b))
                        .send()
                        .await
                        .map_err(S3Error::from)?;
                    self.increment_size(orbit, size).await;
                }
            }
        };
        Ok(hash)
    }
}

#[async_trait]
impl ImmutableDeleteStore for S3BlockStore {
    type Error = S3StoreError;
    async fn remove(&self, orbit: &OrbitId, id: &Hash) -> Result<Option<()>, Self::Error> {
        let size: u64 = match self
            .client
            .get_object_attributes()
            .bucket(&self.bucket)
            .key(self.key(orbit, id))
            .send()
            .await
        {
            Ok(o) if !o.delete_marker() => o.object_size().try_into()?,
            Ok(_) => return Ok(None),
            Err(SdkError::ServiceError {
                err:
                    GetObjectAttributesError {
                        kind: GetObjectAttributesErrorKind::NoSuchKey(_),
                        ..
                    },
                ..
            }) => return Ok(None),
            Err(e) => return Err(S3Error::from(e).into()),
        };
        match self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(self.key(orbit, id))
            .send()
            .await
        {
            Ok(_) => {
                self.decrement_size(orbit, size).await;
                Ok(Some(()))
            }
            // TODO does this distinguish between object missing and object present?
            Err(e) => Err(S3Error::from(e).into()),
        }
    }
}

#[async_trait]
impl StoreSize for S3BlockStore {
    type Error = S3StoreError;
    async fn total_size(&self, orbit: &OrbitId) -> Result<Option<u64>, Self::Error> {
        Ok(self.sizes.get_size(orbit).await)
    }
}
