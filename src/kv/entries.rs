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
mod test {}
