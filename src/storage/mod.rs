use anyhow::{Error, Result};
use aws_sdk_s3::{
    error::{GetObjectError, GetObjectErrorKind},
    types::{ByteStream, SdkError},
};
use aws_smithy_http::body::SdkBody;
use kepler_lib::libipld::cid::{multibase::Base, multihash::Multihash, Cid};
use kepler_lib::resource::OrbitId;
use libp2p::identity::ed25519::Keypair as Ed25519Keypair;
use rocket::tokio::fs;
use std::{collections::HashMap, path::PathBuf, str::FromStr};
use tracing::instrument;

mod dynamodb;
mod either;
mod file_system;
mod indexes;
mod s3;
mod utils;

pub use indexes::KV;

use crate::config;

pub struct StorageUtils {
    config: config::BlockStorage,
}

#[derive(Debug)]
pub struct Repo;

pub type BlockStores =
    either::EitherStore<Box<s3::S3BlockStore>, Box<file_system::FileSystemStore>>;

#[derive(Debug)]
pub enum DataStores {
    S3(Box<s3::S3DataStore>),
    Local(Box<file_system::FileSystemStore>),
}

pub type BlockConfig = either::EitherConfig<s3::S3BlockConfig, file_system::FileSystemConfig>;

#[async_trait]
trait StorageConfig<S> {
    type Error;
    async fn open(&self, orbit: &OrbitId) -> Result<S, Self::Error>;
}

impl StorageUtils {
    pub fn new(config: config::BlockStorage) -> Self {
        Self { config }
    }

    pub async fn healthcheck(&self) -> Result<()> {
        match &self.config {
            config::BlockStorage::S3(r) => {
                let client = s3::S3BlockStore::new_(r, Cid::default());
                // client.init().await
                Ok(())
            }
            config::BlockStorage::Local(r) => {
                if !r.path.is_dir() {
                    Err(anyhow!(
                        "{} does not exist or is not a directory",
                        r.path.to_str().unwrap()
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }

    // TODO either remove this method and only rely on fetching the kp, or find a better way for S3
    pub async fn exists(&self, orbit: Cid) -> Result<bool> {
        match &self.config {
            config::BlockStorage::S3(_) => Ok(self.key_pair(orbit).await?.is_some()),
            config::BlockStorage::Local(r) => {
                let dir = r.path.join(&orbit.to_string_of_base(Base::Base58Btc)?);
                Ok(dir.exists())
            }
        }
    }

    pub async fn relay_key_pair(config: config::BlockStorage) -> Result<Ed25519Keypair> {
        let kp = match config.clone() {
            config::BlockStorage::S3(r) => {
                let client = s3::new_client(r.clone());
                let res = client
                    .get_object()
                    .bucket(r.bucket.clone())
                    .key("kp")
                    .send()
                    .await;
                match res {
                    Ok(o) => Some(Ed25519Keypair::decode(
                        &mut o.body.collect().await?.into_bytes().to_vec(),
                    )?),
                    Err(SdkError::ServiceError {
                        err:
                            GetObjectError {
                                kind: GetObjectErrorKind::NoSuchKey(_),
                                ..
                            },
                        ..
                    }) => None,
                    Err(e) => return Err(e.into()),
                }
            }
            config::BlockStorage::Local(r) => {
                if let Ok(mut bytes) = fs::read(r.path.join("kp")).await {
                    Some(Ed25519Keypair::decode(&mut bytes)?)
                } else {
                    None
                }
            }
        };
        if let Some(k) = kp {
            Ok(k)
        } else {
            let kp = Ed25519Keypair::generate();
            match config {
                config::BlockStorage::S3(r) => {
                    let client = s3::new_client(r.clone());
                    client
                        .put_object()
                        .bucket(r.bucket.clone())
                        .key("kp")
                        .body(ByteStream::new(SdkBody::from(kp.encode().to_vec())))
                        .send()
                        .await?;
                }
                config::BlockStorage::Local(r) => {
                    fs::write(r.path.join("kp"), kp.encode()).await?;
                }
            };
            Ok(kp)
        }
    }

    pub async fn key_pair(&self, orbit: Cid) -> Result<Option<Ed25519Keypair>> {
        match &self.config {
            config::BlockStorage::S3(r) => {
                let client = s3::S3DataStore::new_(r.clone(), orbit);
                Ok(client
                    .get_("keypair".to_string())
                    .await?
                    .map(|mut b| Ed25519Keypair::decode(&mut b))
                    .transpose()?)
            }
            config::BlockStorage::Local(r) => {
                let dir = r.path.join(&orbit.to_string_of_base(Base::Base58Btc)?);
                match fs::read(dir.join("kp")).await {
                    Ok(mut k) => Ok(Some(Ed25519Keypair::decode(&mut k)?)),
                    Err(e) => match e.kind() {
                        std::io::ErrorKind::NotFound => Ok(None),
                        _ => Err(e.into()),
                    },
                }
            }
        }
    }

    pub async fn orbit_id(&self, orbit: Cid) -> Result<Option<OrbitId>> {
        match &self.config {
            config::BlockStorage::S3(r) => {
                let client = s3::S3DataStore::new_(r.clone(), orbit);
                Ok(client
                    .get_("orbit_url".to_string())
                    .await?
                    .map(String::from_utf8)
                    .transpose()?
                    .map(|b| OrbitId::from_str(&b))
                    .transpose()?)
            }
            config::BlockStorage::Local(r) => {
                let dir = r.path.join(&orbit.to_string_of_base(Base::Base58Btc)?);
                match fs::read(dir.join("id")).await {
                    Ok(i) => Ok(Some(String::from_utf8(i)?.parse()?)),
                    Err(e) => match e.kind() {
                        std::io::ErrorKind::NotFound => Ok(None),
                        _ => Err(e.into()),
                    },
                }
            }
        }
    }

    pub async fn setup_orbit(&self, orbit: OrbitId, kp: Ed25519Keypair, auth: &[u8]) -> Result<()> {
        match &self.config {
            config::BlockStorage::S3(r) => {
                let client = s3::S3DataStore::new_(r.clone(), orbit.get_cid());
                client
                    .put_("keypair".to_string(), kp.encode().to_vec())
                    .await?;
                client
                    .put_(
                        "orbit_url".to_string(),
                        orbit.to_string().as_bytes().to_vec(),
                    )
                    .await?;
            }
            config::BlockStorage::Local(r) => {
                let dir = r
                    .path
                    .join(orbit.get_cid().to_string_of_base(Base::Base58Btc)?);
                fs::create_dir_all(&dir)
                    .await
                    .map_err(|e| anyhow!("Couldn't create dir: {}", e))?;

                fs::write(dir.join("access_log"), auth).await?;
                fs::write(dir.join("kp"), kp.encode()).await?;
                fs::write(dir.join("id"), orbit.to_string()).await?;
            }
        };
        Ok(())
    }

    pub async fn ipfs_path(&self, orbit: Cid) -> Result<PathBuf> {
        match &self.config {
            config::BlockStorage::S3(r) => Ok(PathBuf::from(&format!(
                "/s3bucket/{}/s3endpoint/{}/dynamotable/{}/dynamoendpoint/{}/orbitcid/{}",
                r.bucket,
                r.endpoint
                    .as_ref()
                    .map(|e| e.to_string())
                    .unwrap_or_default(),
                r.dynamodb.table,
                r.dynamodb
                    .endpoint
                    .as_ref()
                    .map(|e| e.to_string())
                    .unwrap_or_default(),
                orbit
            ))),
            config::BlockStorage::Local(r) => {
                let path = r.path.join(orbit.to_string_of_base(Base::Base58Btc)?);
                if !path.exists() {
                    fs::create_dir_all(&path).await?;
                }
                Ok(path)
            }
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum VecReadError<E> {
    #[error(transparent)]
    Store(#[from] E),
    #[error(transparent)]
    Read(futures::io::Error),
}

#[async_trait]
pub trait ImmutableStore: Send + Sync {
    type Error: std::error::Error + Send + Sync;
    type Readable: futures::io::AsyncRead + Send + Sync;
    async fn contains(&self, id: &Multihash) -> Result<bool, Self::Error>;
    async fn write(
        &self,
        data: impl futures::io::AsyncRead + Send,
    ) -> Result<Multihash, Self::Error>;
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error>;
    async fn read(&self, id: &Multihash) -> Result<Option<Self::Readable>, Self::Error>;
    async fn read_to_vec(
        &self,
        id: &Multihash,
    ) -> Result<Option<Vec<u8>>, VecReadError<Self::Error>>
    where
        Self::Readable: Send,
    {
        use futures::io::AsyncReadExt;
        let r = match self.read(id).await? {
            None => return Ok(None),
            Some(r) => r,
        };
        let mut v = Vec::new();
        Box::pin(r)
            .read_to_end(&mut v)
            .await
            .map_err(VecReadError::Read)?;
        Ok(Some(v))
    }
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
    ) -> Result<Multihash, Self::Error> {
        self.write(data).await
    }
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error> {
        self.remove(id).await
    }
    async fn read(&self, id: &Multihash) -> Result<Option<Self::Readable>, Self::Error> {
        self.read(id).await
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
