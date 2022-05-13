use anyhow::{Error, Result};
use aws_sdk_s3::{
    error::{GetObjectError, GetObjectErrorKind},
    types::{ByteStream, SdkError},
};
use aws_smithy_http::body::SdkBody;
use ipfs::{
    repo::{
        fs::{FsBlockStore, FsDataStore, FsLock},
        mem::MemLock,
        BlockPut, BlockRm, BlockRmError, BlockStore, Column, DataStore, Lock, LockError, PinKind,
        PinMode, PinStore,
    },
    Block, RepoTypes,
};
use libipld::cid::{multibase::Base, Cid};
use libp2p::identity::ed25519::Keypair as Ed25519Keypair;
use rocket::tokio::fs;
use std::{path::PathBuf, str::FromStr};
use tracing::instrument;

mod dynamodb;
mod indexes;
mod s3;
mod utils;

pub use indexes::KV;

use crate::{config, resource::OrbitId};
use dynamodb::References;

pub struct StorageUtils {
    config: config::BlockStorage,
}

#[derive(Debug)]
pub struct Repo;

#[derive(Debug)]
pub enum BlockStores {
    S3(Box<s3::S3BlockStore>),
    Local(Box<FsBlockStore>),
}

#[derive(Debug)]
pub enum DataStores {
    S3(Box<s3::S3DataStore>),
    Local(Box<FsDataStore>),
}

#[derive(Debug)]
pub enum Locks {
    S3(MemLock),
    Local(FsLock),
}

impl StorageUtils {
    pub fn new(config: config::BlockStorage) -> Self {
        Self { config }
    }

    pub async fn healthcheck(&self) -> Result<()> {
        match &self.config {
            config::BlockStorage::S3(r) => {
                let client = s3::S3BlockStore::new_(r.clone(), Cid::default());
                client.init().await
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
                    tokio::fs::create_dir_all(&path).await?;
                }
                Ok(path)
            }
        }
    }
}

impl RepoTypes for Repo {
    type TBlockStore = BlockStores;
    type TDataStore = DataStores;
    type TLock = Locks;
}

#[async_trait]
impl BlockStore for BlockStores {
    fn new(path: PathBuf) -> Self {
        if path.to_str().unwrap().starts_with("/s3bucket/") {
            Self::S3(Box::new(s3::S3BlockStore::new(path)))
        } else {
            Self::Local(Box::new(FsBlockStore::new(path)))
        }
    }

    async fn init(&self) -> Result<(), Error> {
        match self {
            Self::S3(r) => r.init().await,
            Self::Local(r) => r.init().await,
        }
    }

    async fn open(&self) -> Result<(), Error> {
        match self {
            Self::S3(r) => r.open().await,
            Self::Local(r) => r.open().await,
        }
    }

    async fn contains(&self, cid: &Cid) -> Result<bool, Error> {
        match self {
            Self::S3(r) => r.contains(cid).await,
            Self::Local(r) => r.contains(cid).await,
        }
    }

    #[instrument(name = "blocks::get", skip_all)]
    async fn get(&self, cid: &Cid) -> Result<Option<Block>, Error> {
        match self {
            Self::S3(r) => r.get(cid).await,
            Self::Local(r) => r.get(cid).await,
        }
    }

    #[instrument(name = "blocks::put", skip_all)]
    async fn put(&self, block: Block) -> Result<(Cid, BlockPut), Error> {
        match self {
            Self::S3(r) => r.put(block).await,
            Self::Local(r) => r.put(block).await,
        }
    }

    async fn remove(&self, cid: &Cid) -> Result<Result<BlockRm, BlockRmError>, Error> {
        match self {
            Self::S3(r) => r.remove(cid).await,
            Self::Local(r) => r.remove(cid).await,
        }
    }

    async fn list(&self) -> Result<Vec<Cid>, Error> {
        match self {
            Self::S3(r) => r.list().await,
            Self::Local(r) => r.list().await,
        }
    }

    async fn wipe(&self) {
        match self {
            Self::S3(r) => r.wipe().await,
            Self::Local(r) => r.wipe().await,
        }
    }
}

#[async_trait]
impl DataStore for DataStores {
    fn new(path: PathBuf) -> DataStores {
        if path.to_str().unwrap().starts_with("/s3bucket/") {
            Self::S3(Box::new(s3::S3DataStore::new(path)))
        } else {
            Self::Local(Box::new(FsDataStore::new(path)))
        }
    }

    async fn init(&self) -> Result<(), Error> {
        match self {
            Self::S3(r) => r.init().await,
            Self::Local(r) => r.init().await,
        }
    }

    async fn open(&self) -> Result<(), Error> {
        match self {
            Self::S3(r) => r.open().await,
            Self::Local(r) => r.open().await,
        }
    }

    async fn contains(&self, col: Column, key: &[u8]) -> Result<bool, Error> {
        match self {
            Self::S3(r) => r.contains(col, key).await,
            Self::Local(r) => r.contains(col, key).await,
        }
    }

    async fn get(&self, col: Column, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        match self {
            Self::S3(r) => r.get(col, key).await,
            Self::Local(r) => r.get(col, key).await,
        }
    }

    async fn put(&self, col: Column, key: &[u8], value: &[u8]) -> Result<(), Error> {
        match self {
            Self::S3(r) => r.put(col, key, value).await,
            Self::Local(r) => r.put(col, key, value).await,
        }
    }

    async fn remove(&self, col: Column, key: &[u8]) -> Result<(), Error> {
        match self {
            Self::S3(r) => r.remove(col, key).await,
            Self::Local(r) => r.remove(col, key).await,
        }
    }

    async fn wipe(&self) {
        match self {
            Self::S3(r) => r.wipe().await,
            Self::Local(r) => r.wipe().await,
        }
    }
}

#[async_trait]
impl PinStore for DataStores {
    async fn is_pinned(&self, cid: &Cid) -> Result<bool, Error> {
        match self {
            Self::S3(r) => r.is_pinned(cid).await,
            Self::Local(r) => r.is_pinned(cid).await,
        }
    }

    async fn insert_direct_pin(&self, target: &Cid) -> Result<(), Error> {
        match self {
            Self::S3(r) => r.insert_direct_pin(target).await,
            Self::Local(r) => r.insert_direct_pin(target).await,
        }
    }

    async fn insert_recursive_pin(
        &self,
        target: &Cid,
        referenced: References<'_>,
    ) -> Result<(), Error> {
        match self {
            Self::S3(r) => r.insert_recursive_pin(target, referenced).await,
            Self::Local(r) => r.insert_recursive_pin(target, referenced).await,
        }
    }

    async fn remove_direct_pin(&self, target: &Cid) -> Result<(), Error> {
        match self {
            Self::S3(r) => r.remove_direct_pin(target).await,
            Self::Local(r) => r.remove_direct_pin(target).await,
        }
    }

    async fn remove_recursive_pin(
        &self,
        target: &Cid,
        referenced: References<'_>,
    ) -> Result<(), Error> {
        match self {
            Self::S3(r) => r.remove_recursive_pin(target, referenced).await,
            Self::Local(r) => r.remove_recursive_pin(target, referenced).await,
        }
    }

    async fn list(
        &self,
        requirement: Option<PinMode>,
    ) -> futures::stream::BoxStream<'static, Result<(Cid, PinMode), Error>> {
        match self {
            Self::S3(r) => r.list(requirement).await,
            Self::Local(r) => r.list(requirement).await,
        }
    }

    async fn query(
        &self,
        ids: Vec<Cid>,
        requirement: Option<PinMode>,
    ) -> Result<Vec<(Cid, PinKind<Cid>)>, Error> {
        match self {
            Self::S3(r) => r.query(ids, requirement).await,
            Self::Local(r) => r.query(ids, requirement).await,
        }
    }
}

impl Lock for Locks {
    fn new(path: PathBuf) -> Self {
        if path.to_str().unwrap().starts_with("/s3bucket/") {
            Self::S3(MemLock::new(path))
        } else {
            Self::Local(FsLock::new(path))
        }
    }

    fn try_exclusive(&mut self) -> Result<(), LockError> {
        match self {
            Self::S3(r) => r.try_exclusive(),
            Self::Local(r) => r.try_exclusive(),
        }
    }
}
