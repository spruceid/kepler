use aws_sdk_s3::{
    error::{
        GetObjectError, GetObjectErrorKind, HeadObjectError, HeadObjectErrorKind, PutObjectError,
    },
    types::{ByteStream, SdkError},
    Client, // Config,
    Error as S3Error,
};
use aws_smithy_http::{body::SdkBody, byte_stream::Error as ByteStreamError, endpoint::Endpoint};
use futures::stream::{IntoAsyncRead, MapErr, TryStreamExt};
use kepler_lib::{
    libipld::cid::{
        multibase::{encode, Base},
        multihash::{Code, Error as MultihashError, Multihash},
        Cid,
    },
    resource::OrbitId,
};
use libp2p::identity::{ed25519::Keypair as Ed25519Keypair, error::DecodingError};
use rocket::{async_trait, http::hyper::Uri};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use std::{
    io::Error as IoError,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tempfile::NamedTempFile;
use tokio::fs::File;
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::{
    orbit::ProviderUtils,
    storage::{utils::copy_in, Content, ImmutableStore, KeyedWriteError, StorageConfig},
};

// TODO we could use the same struct for both the block store and the data
// (pin) store, but we need to remember that for now it will be two different
// objects in rust-ipfs
#[derive(Debug, Clone)]
pub struct S3BlockStore {
    // TODO Remove is unused (orbit::delete is never called).
    // When that changes we will need to use a mutex, either local or in Dynamo
    pub client: Client,
    pub bucket: String,
    pub orbit: String,
    size: Arc<AtomicU64>,
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
    async fn open(&self, orbit: &OrbitId) -> Result<Option<S3BlockStore>, Self::Error> {
        Ok(Some(S3BlockStore::new_(self, orbit.get_cid()).await?))
    }
    async fn create(&self, orbit: &OrbitId) -> Result<S3BlockStore, Self::Error> {
        Ok(S3BlockStore::new_(self, orbit.get_cid()).await?)
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
    async fn relay_key_pair(&self) -> Result<Ed25519Keypair, Self::Error> {
        let client = new_client(self);
        match client
            .get_object()
            .bucket(&self.bucket)
            .key("kp")
            .send()
            .await
        {
            Ok(o) => Ok(Ed25519Keypair::decode(
                &mut o.body.collect().await?.into_bytes().to_vec(),
            )?),
            Err(SdkError::ServiceError {
                err:
                    GetObjectError {
                        kind: GetObjectErrorKind::NoSuchKey(_),
                        ..
                    },
                ..
            }) => {
                let kp = Ed25519Keypair::generate();
                client
                    .put_object()
                    .bucket(&self.bucket)
                    .key("kp")
                    .body(ByteStream::new(SdkBody::from(kp.encode().to_vec())))
                    .send()
                    .await?;
                Ok(kp)
            }
            Err(e) => Err(e.into()),
        }
    }
    async fn key_pair(&self, orbit: &OrbitId) -> Result<Option<Ed25519Keypair>, Self::Error> {
        match new_client(self)
            .get_object()
            .bucket(&self.bucket)
            .key(format!("{}/keypair", orbit.get_cid()))
            .send()
            .await
        {
            Ok(o) => Ok(Some(Ed25519Keypair::decode(
                &mut o.body.collect().await?.into_bytes().to_vec(),
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
    async fn setup_orbit(&self, orbit: &OrbitId, key: &Ed25519Keypair) -> Result<(), Self::Error> {
        let client = new_client(self);
        client
            .put_object()
            .bucket(&self.bucket)
            .key(format!("{}/keypair", orbit.get_cid()))
            .body(ByteStream::new(SdkBody::from(key.encode().to_vec())))
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
    let general_config = super::utils::aws_config();
    let sdk_config = aws_sdk_s3::config::Builder::from(&general_config);
    let sdk_config = match &config.endpoint {
        Some(e) => sdk_config.endpoint_resolver(Endpoint::immutable(e.clone())),
        None => sdk_config,
    };
    let sdk_config = sdk_config.build();
    Client::from_conf(sdk_config)
}

impl S3BlockStore {
    async fn new_(config: &S3BlockConfig, orbit: Cid) -> Result<Self, S3Error> {
        let client = new_client(config);
        let size = client
            .list_objects_v2()
            .bucket(&config.bucket)
            .prefix(format!("{}/", orbit))
            .into_paginator()
            .send()
            .try_fold(0, |acc, page| async move {
                Ok(page
                    .contents
                    .into_iter()
                    .flatten()
                    .map(|content| {
                        let s = content.size();
                        if s < 0 {
                            acc
                        } else {
                            acc + s as u64
                        }
                    })
                    .sum::<u64>())
            })
            .await?;
        Ok(S3BlockStore {
            client,
            bucket: config.bucket.clone(),
            orbit: orbit.to_string(),
            size: Arc::new(AtomicU64::new(size)),
        })
    }

    fn key(&self, id: &Multihash) -> String {
        format!("{}/{}", self.orbit, encode(Base::Base64Url, id.to_bytes()))
    }

    fn increment_size(&self, size: u64) {
        self.size.fetch_add(size, Ordering::SeqCst);
    }
    fn decrement_size(&self, size: u64) {
        self.size.fetch_sub(size, Ordering::SeqCst);
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
    Multihash(#[from] MultihashError),
    #[error(transparent)]
    Length(#[from] std::num::TryFromIntError),
}

#[async_trait]
impl ImmutableStore for S3BlockStore {
    type Error = S3StoreError;
    type Readable = IntoAsyncRead<MapErr<ByteStream, fn(ByteStreamError) -> IoError>>;
    async fn contains(&self, id: &Multihash) -> Result<bool, Self::Error> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(self.key(id))
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
    async fn write(
        &self,
        data: impl futures::io::AsyncRead + Send,
        hash_type: Code,
    ) -> Result<Multihash, Self::Error> {
        // TODO find a way to do this without filesystem access
        let (file, path) = NamedTempFile::new()?.into_parts();
        let (multihash, _, written) =
            copy_in(data, File::from_std(file).compat(), hash_type).await?;

        if !self.contains(&multihash).await? {
            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(self.key(&multihash))
                .body(ByteStream::from_path(&path).await?)
                .send()
                .await
                .map_err(S3Error::from)?;
        }
        self.increment_size(written);
        Ok(multihash)
    }
    async fn write_keyed(
        &self,
        data: impl futures::io::AsyncRead + Send,
        hash: &Multihash,
    ) -> Result<(), KeyedWriteError<Self::Error>> {
        if self.contains(hash).await? {
            return Ok(());
        }
        let hash_type = hash
            .code()
            .try_into()
            .map_err(KeyedWriteError::InvalidCode)?;
        let (file, path) = NamedTempFile::new()
            .map_err(S3StoreError::from)?
            .into_parts();
        let (multihash, _, written) = copy_in(data, File::from_std(file).compat(), hash_type)
            .await
            .map_err(S3StoreError::from)?;

        if &multihash != hash {
            return Err(KeyedWriteError::IncorrectHash);
        };

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(self.key(&multihash))
            .body(
                ByteStream::from_path(&path)
                    .await
                    .map_err(S3StoreError::from)?,
            )
            .send()
            .await
            .map_err(|e| KeyedWriteError::Store(S3StoreError::from(S3Error::from(e))))?;
        self.increment_size(written);
        Ok(())
    }
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error> {
        // TODO decrement size on removal
        match self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(self.key(id))
            .send()
            .await
        {
            Ok(_) => Ok(Some(())),
            // TODO does this distinguish between object missing and object present?
            Err(e) => Err(S3Error::from(e).into()),
        }
    }
    async fn read(&self, id: &Multihash) -> Result<Option<Content<Self::Readable>>, Self::Error> {
        let res = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(self.key(id))
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
    async fn total_size(&self) -> Result<u64, Self::Error> {
        Ok(self.size.load(Ordering::Relaxed))
    }
}
