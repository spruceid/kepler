use aws_sdk_s3::{
    error::{
        GetObjectError, GetObjectErrorKind, HeadObjectError, HeadObjectErrorKind, PutObjectError,
    },
    types::{ByteStream, SdkError},
    Client, // Config,
    Error as S3Error,
};
use aws_smithy_http::{body::SdkBody, byte_stream::Error as ByteStreamError, endpoint::Endpoint};
use aws_types::sdk_config::SdkConfig;
use futures::{
    executor::block_on,
    future::Either as AsyncEither,
    stream::{IntoAsyncRead, MapErr, TryStreamExt},
};
use kepler_core::{hash::Hash, storage::*};
use kepler_lib::resource::OrbitId;
use libp2p::identity::{DecodingError, Keypair};
use rocket::{async_trait, http::hyper::Uri};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use std::io::Error as IoError;

use super::file_system;
use crate::orbit::ProviderUtils;

fn aws_config() -> SdkConfig {
    block_on(async { aws_config::from_env().load().await })
}

#[derive(Debug, Clone)]
pub struct S3BlockStore {
    // TODO Remove is unused (orbit::delete is never called).
    // When that changes we will need to use a mutex, either local or in Dynamo
    pub client: Client,
    pub bucket: String,
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
    type Error = std::convert::Infallible;
    async fn open(&self, orbit: &OrbitId) -> Result<Option<S3BlockStore>, Self::Error> {
        Ok(Some(S3BlockStore::new_(self)))
    }
    async fn create(&self, orbit: &OrbitId) -> Result<S3BlockStore, Self::Error> {
        Ok(S3BlockStore::new_(self))
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ProviderError {
    #[error(transparent)]
    S3(#[from] S3Error),
    #[error(transparent)]
    KeypairDecode(#[from] DecodingError),
    #[error(transparent)]
    ByteStream(#[from] ByteStreamError),
}

impl From<SdkError<GetObjectError>> for ProviderError {
    fn from(e: SdkError<GetObjectError>) -> Self {
        Self::S3(e.into())
    }
}

impl From<SdkError<PutObjectError>> for ProviderError {
    fn from(e: SdkError<PutObjectError>) -> Self {
        Self::S3(e.into())
    }
}

#[async_trait]
impl ProviderUtils for S3BlockConfig {
    type Error = ProviderError;
    async fn exists(&self, orbit: &OrbitId) -> Result<bool, Self::Error> {
        self.key_pair(orbit).await.map(|o| o.is_some())
    }
    async fn relay_key_pair(&self) -> Result<Keypair, Self::Error> {
        let client = new_client(self);
        match client
            .get_object()
            .bucket(&self.bucket)
            .key("kp")
            .send()
            .await
        {
            Ok(o) => Ok(Keypair::from_protobuf_encoding(
                &o.body.collect().await?.into_bytes(),
            )?),
            Err(SdkError::ServiceError {
                err:
                    GetObjectError {
                        kind: GetObjectErrorKind::NoSuchKey(_),
                        ..
                    },
                ..
            }) => {
                let kp = Keypair::generate_ed25519();
                client
                    .put_object()
                    .bucket(&self.bucket)
                    .key("kp")
                    .body(ByteStream::new(SdkBody::from(kp.to_protobuf_encoding()?)))
                    .send()
                    .await?;
                Ok(kp)
            }
            Err(e) => Err(e.into()),
        }
    }
    async fn key_pair(&self, orbit: &OrbitId) -> Result<Option<Keypair>, Self::Error> {
        match new_client(self)
            .get_object()
            .bucket(&self.bucket)
            .key(format!("{}/keypair", orbit.get_cid()))
            .send()
            .await
        {
            Ok(o) => Ok(Some(Keypair::from_protobuf_encoding(
                &o.body.collect().await?.into_bytes(),
            )?)),
            Err(SdkError::ServiceError {
                err:
                    GetObjectError {
                        kind: GetObjectErrorKind::NoSuchKey(_),
                        ..
                    },
                ..
            }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    async fn setup_orbit(&self, orbit: &OrbitId, key: &Keypair) -> Result<(), Self::Error> {
        let client = new_client(self);
        client
            .put_object()
            .bucket(&self.bucket)
            .key(format!("{}/keypair", orbit.get_cid()))
            .body(ByteStream::new(SdkBody::from(key.to_protobuf_encoding()?)))
            .send()
            .await?;
        client
            .put_object()
            .bucket(&self.bucket)
            .key(format!("{}/orbit_url", orbit.get_cid()))
            .body(ByteStream::new(SdkBody::from(orbit.to_string())))
            .send()
            .await?;
        Ok(())
    }
}

pub fn new_client(config: &S3BlockConfig) -> Client {
    let general_config = aws_config();
    let sdk_config = aws_sdk_s3::config::Builder::from(&general_config);
    let sdk_config = match &config.endpoint {
        Some(e) => sdk_config.endpoint_resolver(Endpoint::immutable(e.clone())),
        None => sdk_config,
    };
    let sdk_config = sdk_config.build();
    Client::from_conf(sdk_config)
}

impl S3BlockStore {
    pub fn new_(config: &S3BlockConfig) -> Self {
        S3BlockStore {
            client: new_client(config),
            bucket: config.bucket.clone(),
        }
    }

    fn key(&self, orbit: &OrbitId, id: &Hash) -> String {
        format!(
            "{}/{}",
            orbit,
            base64::encode_config(id.as_ref(), base64::URL_SAFE)
        )
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
            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(self.key(orbit, &hash))
                .body(ByteStream::from(f))
                .send()
                .await
                .map_err(S3Error::from)?;
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
        let (_file, path) = f.into_inner();

        if !self.contains(orbit, &hash).await? {
            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(self.key(orbit, &hash))
                .body(ByteStream::from_path(&path).await?)
                .send()
                .await
                .map_err(S3Error::from)?;
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
                    let (_file, path) = t_file.into_inner();
                    self.client
                        .put_object()
                        .bucket(&self.bucket)
                        .key(self.key(orbit, &hash))
                        .body(ByteStream::from_path(&path).await?)
                        .send()
                        .await
                        .map_err(S3Error::from)?;
                }
                AsyncEither::Right(b) => {
                    self.client
                        .put_object()
                        .bucket(&self.bucket)
                        .key(self.key(orbit, &hash))
                        .body(ByteStream::from(b))
                        .send()
                        .await
                        .map_err(S3Error::from)?;
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
        match self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(self.key(orbit, id))
            .send()
            .await
        {
            Ok(_) => Ok(Some(())),
            // TODO does this distinguish between object missing and object present?
            Err(e) => Err(S3Error::from(e).into()),
        }
    }
}
