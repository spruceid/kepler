use super::ImmutableStore;
use std::{
    io::{Error, ErrorKind},
    path::PathBuf,
};
use tokio::{
    fs::{remove_file, File},
    io::copy,
};

pub struct FileSystemStore {
    path: PathBuf,
}

impl FileSystemStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl ImmutableStore for FileSystemStore {
    type Error = std::io::Error;
    type Readable = File;
    async fn write(&self, data: impl futures::io::AsyncRead) -> Result<Cid, Self::Error> {
        todo!();
        // need to stream data through a hasher into the file and return hash
        // match File::open(path.join(cid.to_string())),await {
        //     Ok(f) => copy(data, file).await
        //     Err(e) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        //     Err(e) => Err(e),
        // }
    }
    async fn remove(&self, id: &Cid) -> Result<Option<()>, Self::Error> {
        match remove_file(self.path.join(cid.to_string())).await {
            Ok(()) => Ok(Some(())),
            Err(e) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    async fn read(&self, id: &Cid) -> Result<Option<Self::Readable>, Self::Error> {
        match File::open(path.join(cid.to_string())).await {
            Ok(f) => Ok(Some(f)),
            Err(e) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}
