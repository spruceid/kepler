use super::name::{to_block, to_block_raw};
use anyhow::Result;
use libipld::{cid::Cid, Block, DagCbor, DefaultParams};
use std::collections::BTreeMap;

#[derive(DagCbor)]
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

#[derive(DagCbor)]
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

#[derive(DagCbor)]
pub struct DataTree {
    pub order: u64,
    pub chunk: Vec<u8>,
    pub children: Vec<Cid>,
}
