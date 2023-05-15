use crate::orbit::ProviderUtils;
use kepler_core::{hash::Hash, storage::*};
use kepler_lib::resource::OrbitId;
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

    fn get_path(&self, mh: &Hash) -> PathBuf {
        self.path
            .join(base64::encode_config(mh.as_ref(), base64::URL_SAFE))
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
    Persist(#[from] PathPersistError),
}

#[async_trait]
impl ImmutableReadStore for FileSystemStore {
    type Error = FileSystemStoreError;
    type Readable = Compat<File>;
    async fn contains(&self, id: &Hash) -> Result<bool, Self::Error> {
        Ok(self.get_path(id).exists())
    }
    async fn read(&self, id: &Hash) -> Result<Option<Content<Self::Readable>>, Self::Error> {
        match File::open(self.get_path(id)).await {
            Ok(f) => Ok(Some(Content::new(f.metadata().await?.len(), f.compat()))),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

pub struct TempFileSystemStage;

pub struct TempFileStage(Compat<File>, tempfile::TempPath);

impl TempFileSystemStage {
    fn new(file: NamedTempFile) -> Self {
        let (f, p) = file.into_parts();
        Self(f.compat(), p)
    }
    fn into_inner(self) -> (Compat<File>, tempfile::TempPath) {
        (self.0, self.1)
    }
}

#[async_trait]
impl ImmutableStaging for TempFileSystemStage {
    type Error = FileSystemStoreError;
    type Writable = TempFileStage;
    async fn get_staging_buffer(&self) -> Result<Self::Writable, Self::Error> {
        Ok(TempFileStage::new(NamedTempFile::new()?))
    }
}

#[async_trait]
impl ImmutableWriteStore<TempFileSystemStage> for FileSystemStore {
    type Error = FileSystemStoreError;
    async fn persist(
        &self,
        staged: HashBuffer<TempFileSystemStage::Writable>,
    ) -> Result<Hash, Self::Error> {
        let (h, f) = staged.into_inner();
        let (_, path) = f.into_inner();

        if !self.contains(&h).await? {
            path.persist(self.get_path(&h))?;
        }
        Ok(h)
    }
    async fn remove(&self, id: &Hash) -> Result<Option<()>, Self::Error> {
        match remove_file(self.get_path(id)).await {
            Ok(()) => Ok(Some(())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[async_trait]
impl ImmutableWriteStore<memory::MemoryStaging> for FileSystemStore {
    type Error = FileSystemStoreError;
    async fn persist(
        &self,
        staged: HashBuffer<memory::MemoryStaging::Writable>,
    ) -> Result<Hash, Self::Error> {
        let (h, v) = staged.into_inner();
        if !self.contains(&h).await? {
            let file = File::open(self.get_path(h)).await?;
            let mut writer = futures::io::BufWriter::new(file);
            writer.write_all(&v).await?;
        }
        Ok(h)
    }
    async fn remove(&self, id: &Hash) -> Result<Option<()>, Self::Error> {
        match remove_file(self.get_path(id)).await {
            Ok(()) => Ok(Some(())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[async_trait]
impl ImmutableWriteStore<either::Either<TempFileSystemStage, memory::MemoryStaging>>
    for FileSystemStore
{
    type Error = FileSystemStoreError;
    async fn persist(
        &self,
        staged: HashBuffer<either::Either<TempFileSystemStage, memory::MemoryStaging>::Writable>,
    ) -> Result<Hash, Self::Error> {
        let (h, f) = staged.into_inner();

        if !self.contains(&h).await? {
            match f {
                either::Either::A(t_file) => {
                    let (_, path) = f.into_inner();
                    path.persist(self.get_path(&h))?;
                }
                either::Either::B(v) => {
                    let file = File::open(self.get_path(h)).await?;
                    let mut writer = futures::io::BufWriter::new(file);
                    writer.write_all(&v).await?;
                }
            }
        };
        Ok(h)
    }
}

#[async_trait]
impl ImmutableDeleteStore for FileSystemStore {
    type Error = FileSystemStoreError;
    async fn remove(&self, id: &Hash) -> Result<Option<()>, Self::Error> {
        match remove_file(self.get_path(id)).await {
            Ok(()) => Ok(Some(())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
