use super::name::{to_block, to_block_raw};
use anyhow::Result;
use libipld::{cid::Cid, Block, DagCbor, DefaultParams};
use std::collections::BTreeMap;

#[derive(DagCbor, PartialEq, Debug)]
pub struct S3Object {
    pub data: S3ObjectData,
    pub version_id: String,
}

impl S3Object {
    pub fn new(data: S3ObjectData, version_id: String) -> Self {
        Self { data, version_id }
    }

    pub fn to_block(&self) -> Result<Block<DefaultParams>> {
        to_block(self)
    }
}

#[derive(DagCbor, PartialEq, Debug)]
pub struct S3ObjectData {
    pub key: Vec<u8>,
    pub value: Cid,
    pub metadata: BTreeMap<String, String>,
}

impl S3ObjectData {
    pub fn new(
        key: Vec<u8>,
        value: Cid,
        metadata: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        Self {
            key,
            value,
            metadata: metadata.into_iter().collect(),
        }
    }
}

pub struct S3ObjectBuilder {
    pub key: Vec<u8>,
    pub metadata: BTreeMap<String, String>,
}

impl S3ObjectBuilder {
    pub fn new(key: Vec<u8>, metadata: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            key,
            metadata: metadata.into_iter().collect(),
        }
    }

    pub fn add_content(self, value: Cid, priority: u64) -> Result<S3Object> {
        let d = S3ObjectData::new(self.key, value, self.metadata);
        let d_cid = *to_block(&d)?.cid();
        let version_id = format!("{}.{}", priority, d_cid);
        Ok(S3Object::new(d, version_id))
    }
}

#[derive(DagCbor)]
pub struct DataTree {
    pub order: u64,
    pub chunk: Vec<u8>,
    pub children: Vec<Cid>,
}
