use crate::{
    orbit::ProviderUtils,
    storage::{utils::copy_in, Content, ImmutableStore, KeyedWriteError, StorageConfig},
};
use kepler_lib::{
    libipld::cid::{
        multibase::{encode, Base},
        multihash::{Code, Error as MultihashError, Multihash},
    },
    resource::OrbitId,
};
use libp2p::identity::{ed25519::Keypair as Ed25519Keypair, error::DecodingError};
use serde::{Deserialize, Serialize};
use std::{
    io::{Error as IoError, ErrorKind},
    path::{Path, PathBuf},
};
use tempfile::{NamedTempFile, PathPersistError};
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

impl FileSystemConfig {
    pub fn new<P: AsRef<Path>>(p: P) -> Self {
        Self {
            path: p.as_ref().into(),
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
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
    Persist(#[from] PathPersistError),
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
        hash_type: Code,
    ) -> Result<Multihash, Self::Error> {
        let (file, path) = NamedTempFile::new()?.into_parts();
        let (multihash, _) = copy_in(data, File::from_std(file).compat(), hash_type).await?;

        if !self.contains(&multihash).await? {
            path.persist(self.get_path(&multihash))?;
        }
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
            .map_err(FileSystemStoreError::Io)?
            .into_parts();
        let (multihash, _) = copy_in(data, File::from_std(file).compat(), hash_type)
            .await
            .map_err(FileSystemStoreError::from)?;

        if &multihash != hash {
            return Err(KeyedWriteError::IncorrectHash);
        };

        path.persist(self.get_path(&multihash))
            .map_err(FileSystemStoreError::from)?;
        Ok(())
    }
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error> {
        match remove_file(self.get_path(id)).await {
            Ok(()) => Ok(Some(())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    async fn read(&self, id: &Multihash) -> Result<Option<Content<Self::Readable>>, Self::Error> {
        match File::open(self.get_path(id)).await {
            Ok(f) => Ok(Some(Content::new(f.metadata().await?.len(), f.compat()))),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
