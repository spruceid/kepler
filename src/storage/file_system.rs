use super::ImmutableStore;
use kepler_lib::libipld::cid::{
    multibase::{encode, Base},
    multihash::Multihash,
};
use std::{io::ErrorKind, path::PathBuf};
use tokio::{
    fs::{remove_file, File},
    io::copy,
};
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

#[derive(Debug)]
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
