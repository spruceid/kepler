use super::{ImmutableStore, StorageConfig};
use kepler_lib::{
    libipld::cid::{
        multibase::{encode, Base},
        multihash::Multihash,
    },
    resource::OrbitId,
};
use serde::{Deserialize, Serialize};
use std::{io::ErrorKind, path::PathBuf};
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
    type Error = std::io::Error;
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

#[async_trait]
impl ImmutableStore for FileSystemStore {
    type Error = std::io::Error;
    type Readable = Compat<File>;
    async fn contains(&self, id: &Multihash) -> Result<bool, Self::Error> {
        Ok(self.get_path(id).exists())
    }
    async fn write(
        &self,
        data: impl futures::io::AsyncRead + Send,
    ) -> Result<Multihash, Self::Error> {
        // TODO lock file to prevent overlapping writes
        // only open to write if not existing AND not being written to right now
        todo!();
        // write into tmp then rename, to name after the hash
        // need to stream data through a hasher into the file and return hash
        // match File::open(path.join(cid.to_string())),await {
        //     Ok(f) => copy(data, file).await
        //     Err(e) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        //     Err(e) => Err(e),
        // }
    }
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error> {
        match remove_file(self.get_path(id)).await {
            Ok(()) => Ok(Some(())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    async fn read(&self, id: &Multihash) -> Result<Option<Self::Readable>, Self::Error> {
        match File::open(self.get_path(id)).await {
            Ok(f) => Ok(Some(f.compat())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}
