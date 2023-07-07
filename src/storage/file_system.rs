use super::size::OrbitSizes;
use core::pin::Pin;
use futures::{
    future::{Either as AsyncEither, TryFutureExt},
    io::{AsyncWrite, AsyncWriteExt},
    stream::TryStreamExt,
    task::{Context, Poll},
};
use kepler_core::{hash::Hash, storage::*};
use kepler_lib::resource::OrbitId;
use pin_project::pin_project;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{Error as IoError, ErrorKind},
    path::{Path, PathBuf},
};
use tempfile::{NamedTempFile, PathPersistError};
use tokio::fs::{create_dir_all, metadata, remove_file, File};
use tokio_stream::wrappers::ReadDirStream;

use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

#[derive(Debug, Clone)]
pub struct FileSystemStore {
    path: PathBuf,
    sizes: OrbitSizes,
}

impl FileSystemStore {
    async fn new(path: PathBuf) -> Result<Self, IoError> {
        // get the size of the directory
        let sizes = store_sizes(&path).await?.into();
        Ok(Self { path, sizes })
    }

    fn get_path(&self, orbit: &OrbitId, mh: &Hash) -> PathBuf {
        self.path
            .join(orbit.suffix())
            .join(orbit.name())
            .join(base64::encode_config(mh.as_ref(), base64::URL_SAFE))
    }

    async fn increment_size(&self, orbit: &OrbitId, size: u64) {
        self.sizes.increment_size(orbit, size).await;
    }
    async fn decrement_size(&self, orbit: &OrbitId, size: u64) {
        self.sizes.decrement_size(orbit, size).await;
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
            Ok(FileSystemStore::new(self.path.clone()).await?)
        } else {
            Err(IoError::new(ErrorKind::NotFound, "path is not a directory"))
        }
    }
}

#[async_trait]
impl StorageSetup for FileSystemStore {
    type Error = IoError;
    async fn create(&self, orbit: &OrbitId) -> Result<(), Self::Error> {
        let path = self.path.join(orbit.suffix()).join(orbit.name());
        if !path.is_dir() {
            create_dir_all(&path).await?;
        }
        self.sizes.init_size(orbit.clone()).await;
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

#[async_trait]
impl StoreSize for FileSystemStore {
    type Error = FileSystemStoreError;
    async fn total_size(&self, orbit: &OrbitId) -> Result<Option<u64>, Self::Error> {
        Ok(self.sizes.get_size(orbit).await)
    }
}

// get the sum size of all files in this directory (recurse into subdirectories with orbit ID names)
async fn store_sizes<P: AsRef<Path>>(path: &P) -> Result<HashMap<OrbitId, u64>, IoError> {
    ReadDirStream::new(tokio::fs::read_dir(path).await?)
        // for every entry in the store dir
        .try_fold(HashMap::new(), |mut acc, entry| async move {
            // if its a directory and the suffix is a valid string
            if let (true, Ok(ref suffix)) = (
                entry.metadata().await?.is_dir(),
                entry.file_name().into_string(),
            ) {
                let mut ds = ReadDirStream::new(tokio::fs::read_dir(entry.path()).await?);
                // go through each suffix directory
                while let Some(entry) = ds.try_next().await? {
                    // for each entry in the suffix directory
                    // if its a directory and the name is a valid string
                    if let (true, Ok(name)) = (
                        entry.metadata().await?.is_dir(),
                        entry.file_name().into_string(),
                    ) {
                        // get the orbit ID from suffix and name
                        let orbit = OrbitId::new(suffix.clone(), name);
                        let size = orbit_size(&entry.path()).await?;
                        acc.insert(orbit, size);
                    }
                }
            };
            Ok(acc)
        })
        .await
}

async fn orbit_size<P: AsRef<Path>>(path: &P) -> Result<u64, IoError> {
    // get the sum size of all files in this directory (do not recurse into subdirectories)
    ReadDirStream::new(tokio::fs::read_dir(path).await?)
        .try_fold(0, |acc, entry| async move {
            entry
                .metadata()
                .map_ok(|m| if m.is_dir() { acc } else { acc + m.len() })
                .await
        })
        .await
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

    pub async fn size(&self) -> Result<u64, IoError> {
        Ok(self.0.get_ref().metadata().await?.len())
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

        let hash = h.finalize();
        if !self.contains(orbit, &hash).await? {
            let size = f.size().await?;
            let (_, path) = f.into_inner();
            path.persist(self.get_path(orbit, &hash))?;
            self.increment_size(orbit, size).await;
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
            let size = v.len() as u64;
            let mut writer = futures::io::BufWriter::new(file.compat());
            writer.write_all(&v).await?;
            writer.flush().await?;
            self.increment_size(orbit, size).await;
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
                    let size = t_file.size().await?;
                    let (_, path) = t_file.into_inner();
                    path.persist(self.get_path(orbit, &hash))?;
                    self.increment_size(orbit, size).await;
                }
                AsyncEither::Right(v) => {
                    let file = File::create(self.get_path(orbit, &hash)).await?;
                    let size = v.len() as u64;
                    let mut writer = futures::io::BufWriter::new(file.compat());
                    writer.write_all(&v).await?;
                    writer.flush().await?;
                    self.increment_size(orbit, size).await;
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
        let path = self.get_path(orbit, id);
        let size = metadata(&path).await?.len();
        match remove_file(path).await {
            Ok(()) => {
                self.decrement_size(orbit, size).await;
                Ok(Some(()))
            }
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

#[cfg(test)]
mod test {
    use super::*;
    use futures::io::AsyncReadExt;

    #[test]
    async fn test_file_system_store() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = FileSystemConfig::new(dir.path().to_path_buf());
        let store = cfg
            .create(&"kepler:example://default".parse().unwrap())
            .await
            .unwrap();
        let data = b"hello world";
        assert_eq!(store.total_size().await.unwrap(), 0);
        let hash = store.write(&data[..], Code::Sha2_256).await.unwrap();

        assert_eq!(store.contains(&hash).await.unwrap(), true);
        assert_eq!(store.total_size().await.unwrap(), data.len() as u64);

        let mut buf = Vec::new();
        store
            .read(&hash)
            .await
            .unwrap()
            .unwrap()
            .read_to_end(&mut buf)
            .await
            .unwrap();

        assert_eq!(buf, data);
        assert_eq!(store.read_to_vec(&hash).await.unwrap().unwrap(), data);
        assert_eq!(store.remove(&hash).await.unwrap(), Some(()));
        assert_eq!(store.remove(&hash).await.unwrap(), None);
        assert_eq!(store.contains(&hash).await.unwrap(), false);
        assert_eq!(store.total_size().await.unwrap(), 0);
        assert_eq!(store.read(&hash).await.unwrap().map(|_| ()), None);
    }
}
