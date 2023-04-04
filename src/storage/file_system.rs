use crate::{
    orbit::ProviderUtils,
    storage::{utils::copy_in, Content, ImmutableStore, KeyedWriteError, StorageConfig},
};
use futures::{future::TryFutureExt, stream::TryStreamExt};
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
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tempfile::{NamedTempFile, PathPersistError};
use tokio::fs::{create_dir_all, metadata, read, remove_file, write, File};
use tokio_stream::wrappers::ReadDirStream;

use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

#[derive(Debug, Clone)]
pub struct FileSystemStore {
    path: PathBuf,
    size: Arc<AtomicU64>,
}

impl FileSystemStore {
    fn new(path: PathBuf, size: u64) -> Self {
        Self {
            path,
            size: Arc::new(AtomicU64::new(size)),
        }
    }

    fn get_path(&self, mh: &Multihash) -> PathBuf {
        self.path.join(encode(Base::Base64Url, mh.to_bytes()))
    }

    fn increment_size(&self, size: u64) {
        self.size.fetch_add(size, Ordering::SeqCst);
    }
    fn decrement_size(&self, size: u64) {
        self.size.fetch_sub(size, Ordering::SeqCst);
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
        if !path.is_dir() {
            return Ok(None);
        }
        // get the size of the directory
        let size = dir_size(&path).await?;
        Ok(Some(FileSystemStore::new(path, size)))
    }
    async fn create(&self, orbit: &OrbitId) -> Result<FileSystemStore, Self::Error> {
        let path = self.path.join(orbit.get_cid().to_string()).join("blocks");
        let size = if !path.is_dir() {
            create_dir_all(&path).await?;
            0
        } else {
            dir_size(&path).await?
        };
        Ok(FileSystemStore::new(path, size))
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
        let (multihash, _, written) =
            copy_in(data, File::from_std(file).compat(), hash_type).await?;

        self.increment_size(written);

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
        let (multihash, _, written) = copy_in(data, File::from_std(file).compat(), hash_type)
            .await
            .map_err(FileSystemStoreError::from)?;

        if &multihash != hash {
            return Err(KeyedWriteError::IncorrectHash);
        };

        path.persist(self.get_path(&multihash))
            .map_err(FileSystemStoreError::from)?;
        self.increment_size(written);
        Ok(())
    }
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error> {
        let path = self.get_path(id);
        let size = match metadata(&path).await {
            Ok(m) => m.len(),
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        // this shouldnt be not found, because we checked earlier when getting the size
        remove_file(self.get_path(id)).await?;
        self.decrement_size(size);
        Ok(Some(()))
    }
    async fn read(&self, id: &Multihash) -> Result<Option<Content<Self::Readable>>, Self::Error> {
        match File::open(self.get_path(id)).await {
            Ok(f) => Ok(Some(Content::new(f.metadata().await?.len(), f.compat()))),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    async fn total_size(&self) -> Result<u64, Self::Error> {
        Ok(self.size.load(Ordering::Relaxed))
    }
}

async fn dir_size<P: AsRef<Path>>(path: &P) -> Result<u64, IoError> {
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
