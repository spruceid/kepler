use super::name::{to_block, to_block_raw};
use anyhow::Result;
use libipld::{cid::Cid, Block, DagCbor, DefaultParams};
use std::collections::BTreeMap;

#[derive(DagCbor, PartialEq, Debug, Clone)]
pub struct S3Object {
    pub key: Vec<u8>,
    pub value: Cid,
    pub metadata: BTreeMap<String, String>,
}

impl S3Object {
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

    pub fn to_block(&self) -> Result<Block<DefaultParams>> {
        to_block(self)
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
        Ok(S3Object::new(self.key, value, self.metadata))
    }
}

#[derive(DagCbor)]
pub struct DataTree {
    pub order: u64,
    pub chunk: Vec<u8>,
    pub children: Vec<Cid>,
}
