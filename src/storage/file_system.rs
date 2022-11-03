use crate::{
    orbit::ProviderUtils,
    storage::{utils::HashBuffer, ImmutableStore, StorageConfig},
};
use futures::io::{copy, AllowStdIo};
use kepler_lib::{
    libipld::cid::{
        multibase::{encode, Base},
        multihash::{
            Blake3Hasher, Code, Error as MultihashError, Hasher, Multihash, MultihashDigest,
        },
    },
    resource::OrbitId,
};
use libp2p::identity::{ed25519::Keypair as Ed25519Keypair, error::DecodingError};
use serde::{Deserialize, Serialize};
use std::{
    io::{Error as IoError, ErrorKind},
    path::PathBuf,
};
use tempfile::{NamedTempFile, PersistError};
use tokio::fs::{create_dir_all, read, remove_file, write, File};
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

#[derive(Debug, Clone)]
pub struct FileSystemStore {
    path: PathBuf,
}

impl FileSystemStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn get_path(&self, mh: &Multihash) -> PathBuf {
        self.path.join(encode(Base::Base64Url, mh.to_bytes()))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct FileSystemConfig {
    path: PathBuf,
}

#[async_trait]
impl StorageConfig<FileSystemStore> for FileSystemConfig {
    type Error = IoError;
    async fn open(&self, orbit: &OrbitId) -> Result<Option<FileSystemStore>, Self::Error> {
        let path = self.path.join(orbit.get_cid().to_string()).join("blocks");
        if path.is_dir() {
            Ok(Some(FileSystemStore::new(path)))
        } else {
            Ok(None)
        }
    }
    async fn create(&self, orbit: &OrbitId) -> Result<FileSystemStore, Self::Error> {
        let path = self.path.join(orbit.get_cid().to_string()).join("blocks");
        if !path.is_dir() {
            create_dir_all(&path).await?;
        }
        Ok(FileSystemStore::new(path))
    }
}

impl Default for FileSystemConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from(r"/tmp/kepler/blocks"),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ProviderError {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    KeypairDecode(#[from] DecodingError),
}

#[async_trait]
impl ProviderUtils for FileSystemConfig {
    type Error = ProviderError;
    async fn exists(&self, orbit: &OrbitId) -> Result<bool, Self::Error> {
        Ok(self
            .path
            .join(orbit.get_cid().to_string())
            .join("blocks")
            .is_dir())
    }
    async fn relay_key_pair(&self) -> Result<Ed25519Keypair, Self::Error> {
        let path = self.path.join("kp");
        match read(&path).await {
            Ok(mut k) => Ok(Ed25519Keypair::decode(&mut k)?),
            Err(e) if e.kind() == ErrorKind::NotFound => {
                let k = Ed25519Keypair::generate();
                write(&path, k.encode()).await?;
                Ok(k)
            }
            Err(e) => Err(e.into()),
        }
    }
    async fn key_pair(&self, orbit: &OrbitId) -> Result<Option<Ed25519Keypair>, Self::Error> {
        match read(self.path.join(orbit.get_cid().to_string()).join("kp")).await {
            Ok(mut k) => Ok(Some(Ed25519Keypair::decode(&mut k)?)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    async fn setup_orbit(&self, orbit: &OrbitId, key: &Ed25519Keypair) -> Result<(), Self::Error> {
        let dir = self.path.join(orbit.get_cid().to_string());
        create_dir_all(&dir).await?;
        write(dir.join("kp"), key.encode()).await?;
        write(dir.join("id"), orbit.to_string()).await?;
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum FileSystemStoreError {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    Multihash(#[from] MultihashError),
    #[error(transparent)]
    Persist(#[from] PersistError),
}

#[async_trait]
impl ImmutableStore for FileSystemStore {
    type Error = FileSystemStoreError;
    type Readable = Compat<File>;
    async fn contains(&self, id: &Multihash) -> Result<bool, Self::Error> {
        Ok(self.get_path(id).exists())
    }
    async fn write(
        &self,
        data: impl futures::io::AsyncRead + Send,
    ) -> Result<Multihash, Self::Error> {
        let mut hb = HashBuffer::<Blake3Hasher<32>, AllowStdIo<NamedTempFile>>::new(
            AllowStdIo::new(NamedTempFile::new_in(&self.path)?),
        );

        copy(data, &mut hb).await?;

        let (mut hasher, file) = hb.into_inner();

        let multihash = Code::Blake3_256.wrap(hasher.finalize())?;

        let file = file.into_inner();
        file.persist(self.get_path(&multihash))?;
        Ok(multihash)
    }
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error> {
        match remove_file(self.get_path(id)).await {
            Ok(()) => Ok(Some(())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    async fn read(&self, id: &Multihash) -> Result<Option<Self::Readable>, Self::Error> {
        match File::open(self.get_path(id)).await {
            Ok(f) => Ok(Some(f.compat())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
