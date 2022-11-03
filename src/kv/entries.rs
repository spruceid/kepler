use super::to_block;
use crate::Block;
use anyhow::Result;
use kepler_lib::libipld::{cid::Cid, DagCbor};
use std::collections::BTreeMap;

#[derive(DagCbor, PartialEq, Eq, Debug, Clone)]
pub struct Object {
    pub key: Vec<u8>,
    pub value: Cid,
    pub metadata: BTreeMap<String, String>,
    pub auth: Cid,
}

impl Object {
    pub fn new(
        key: Vec<u8>,
        value: Cid,
        metadata: impl IntoIterator<Item = (String, String)>,
        auth: Cid,
    ) -> Self {
        Self {
            key,
            value,
            metadata: metadata.into_iter().collect(),
            auth,
        }
    }

    pub fn to_block(&self) -> Result<Block> {
        to_block(self)
    }
}

pub struct ObjectBuilder {
    pub key: Vec<u8>,
    pub metadata: BTreeMap<String, String>,
    pub auth: Cid,
}

impl ObjectBuilder {
    pub fn new(
        key: Vec<u8>,
        metadata: impl IntoIterator<Item = (String, String)>,
        auth: Cid,
    ) -> Self {
        Self {
            key,
            metadata: metadata.into_iter().collect(),
            auth,
        }
    }

    pub fn add_content(self, value: Cid) -> Object {
        Object::new(self.key, value, self.metadata, self.auth)
    }
}

#[cfg(test)]
mod test {

    // use super::*;
    // use crate::{config, kv::DagCborCodec, tracing::tracing_try_init};

    // #[tokio::test(flavor = "multi_thread")]
    // async fn write() -> Result<(), anyhow::Error> {
    //     tracing_try_init(&config::Logging::default());
    //     let tmp = tempfile::TempDir::new("test_streams")?;
    //     let data = vec![3u8; 1000000000 * 3];

    //     let mut config = IpfsOptions::inmemory_with_generated_keys();
    //     config.ipfs_path = tmp.path().into();

    //     let (ipfs, task) = config.create_uninitialised_ipfs()?.start().await?;
    //     let _join_handle = tokio::spawn(task);

    //     let o = write_to_store(&ipfs, Cursor::new(data.clone())).await?;

    //     let content = ipfs
    //         .get_block(&o)
    //         .await?
    //         .decode::<DagCborCodec, Vec<(Cid, u32)>>()?;

    //     let mut read = read_from_store(ipfs, content);

    //     let mut out = Vec::new();
    //     tokio::io::copy(&mut read, &mut out).await?;

    //     assert_eq!(out.len(), data.len());
    //     Ok(())
    // }
}
