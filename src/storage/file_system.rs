use core::pin::Pin;
use futures::{
    future::Either as AsyncEither,
    io::{AsyncWrite, AsyncWriteExt},
    task::{Context, Poll},
};
use kepler_core::{hash::Hash, storage::*};
use kepler_lib::resource::OrbitId;
use pin_project::pin_project;
use serde::{Deserialize, Serialize};
use std::{
    io::{Error as IoError, ErrorKind},
    path::{Path, PathBuf},
};
use tempfile::{NamedTempFile, PathPersistError};
use tokio::fs::{create_dir_all, remove_file, File};

use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

#[derive(Debug, Clone)]
pub struct FileSystemStore {
    path: PathBuf,
}

impl FileSystemStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn get_path(&self, orbit: &OrbitId, mh: &Hash) -> PathBuf {
        self.path
            .join(orbit.to_string())
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
    async fn open(&self) -> Result<FileSystemStore, Self::Error> {
        if self.path.is_dir() {
            Ok(FileSystemStore::new(self.path.clone()))
        } else {
            Err(IoError::new(ErrorKind::NotFound, "path is not a directory"))
        }
    }
}

#[async_trait]
impl StorageSetup for FileSystemStore {
    type Error = IoError;
    async fn create(&self, orbit: &OrbitId) -> Result<(), Self::Error> {
        let path = self.path.join(orbit.to_string());
        if !path.is_dir() {
            create_dir_all(&path).await?;
        }
        Ok(())
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
    async fn contains(&self, orbit: &OrbitId, id: &Hash) -> Result<bool, Self::Error> {
        Ok(self.get_path(orbit, id).exists())
    }
    async fn read(
        &self,
        orbit: &OrbitId,
        id: &Hash,
    ) -> Result<Option<Content<Self::Readable>>, Self::Error> {
        match File::open(self.get_path(orbit, id)).await {
            Ok(f) => Ok(Some(Content::new(f.metadata().await?.len(), f.compat()))),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[derive(Default, Debug, Clone, Hash, PartialEq, Eq)]
pub struct TempFileSystemStage;

#[pin_project]
#[derive(Debug)]
pub struct TempFileStage(#[pin] Compat<File>, tempfile::TempPath);

impl TempFileStage {
    pub fn new(file: NamedTempFile) -> Self {
        let (f, p) = file.into_parts();
        Self(File::from_std(f).compat(), p)
    }
    pub fn into_inner(self) -> (Compat<File>, tempfile::TempPath) {
        (self.0, self.1)
    }
}

impl AsyncWrite for TempFileStage {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, IoError>> {
        self.project().0.poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        self.project().0.poll_flush(cx)
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        self.project().0.poll_close(cx)
    }
}

#[async_trait]
impl ImmutableStaging for TempFileSystemStage {
    type Error = FileSystemStoreError;
    type Writable = TempFileStage;
    async fn get_staging_buffer(&self, _: &OrbitId) -> Result<Self::Writable, Self::Error> {
        Ok(TempFileStage::new(NamedTempFile::new()?))
    }
}

#[async_trait]
impl ImmutableWriteStore<TempFileSystemStage> for FileSystemStore {
    type Error = FileSystemStoreError;
    async fn persist(
        &self,
        orbit: &OrbitId,
        staged: HashBuffer<<TempFileSystemStage as ImmutableStaging>::Writable>,
    ) -> Result<Hash, Self::Error> {
        let (mut h, f) = staged.into_inner();
        let (_, path) = f.into_inner();

        let hash = h.finalize();
        if !self.contains(orbit, &hash).await? {
            path.persist(self.get_path(orbit, &hash))?;
        }
        Ok(hash)
    }
}

#[async_trait]
impl ImmutableWriteStore<memory::MemoryStaging> for FileSystemStore {
    type Error = FileSystemStoreError;
    async fn persist(
        &self,
        orbit: &OrbitId,
        staged: HashBuffer<<memory::MemoryStaging as ImmutableStaging>::Writable>,
    ) -> Result<Hash, Self::Error> {
        let (mut h, v) = staged.into_inner();
        let hash = h.finalize();
        if !self.contains(orbit, &hash).await? {
            let file = File::create(self.get_path(orbit, &hash)).await?;
            let mut writer = futures::io::BufWriter::new(file.compat());
            writer.write_all(&v).await?;
            writer.flush().await?;
        }
        Ok(hash)
    }
}

#[async_trait]
impl ImmutableWriteStore<either::Either<TempFileSystemStage, memory::MemoryStaging>>
    for FileSystemStore
{
    type Error = FileSystemStoreError;
    async fn persist(
        &self,
        orbit: &OrbitId,
        staged: HashBuffer<<either::Either<TempFileSystemStage, memory::MemoryStaging> as ImmutableStaging>::Writable>,
    ) -> Result<Hash, Self::Error> {
        let (mut h, f) = staged.into_inner();
        let hash = h.finalize();

        if !self.contains(orbit, &hash).await? {
            match f {
                AsyncEither::Left(t_file) => {
                    let (_, path) = t_file.into_inner();
                    path.persist(self.get_path(orbit, &hash))?;
                }
                AsyncEither::Right(v) => {
                    let file = File::create(self.get_path(orbit, &hash)).await?;
                    let mut writer = futures::io::BufWriter::new(file.compat());
                    writer.write_all(&v).await?;
                    writer.flush().await?;
                }
            }
        };
        Ok(hash)
    }
}

#[async_trait]
impl ImmutableDeleteStore for FileSystemStore {
    type Error = FileSystemStoreError;
    async fn remove(&self, orbit: &OrbitId, id: &Hash) -> Result<Option<()>, Self::Error> {
        match remove_file(self.get_path(orbit, id)).await {
            Ok(()) => Ok(Some(())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[async_trait]
impl StorageConfig<TempFileSystemStage> for TempFileSystemStage {
    type Error = std::convert::Infallible;
    async fn open(&self) -> Result<TempFileSystemStage, Self::Error> {
        Ok(Self)
    }
}
