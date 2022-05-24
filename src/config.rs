use crate::allow_list::OrbitAllowListService;
use rocket::http::hyper::Uri;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, PartialEq, Eq)]
pub struct Config {
    pub storage: Storage,
    pub orbits: OrbitsConfig,
    pub relay: Relay,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, PartialEq, Eq)]
pub struct OrbitsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowlist: Option<OrbitAllowListService>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, PartialEq, Eq)]
pub struct Storage {
    pub blocks: BlockStorage,
    pub indexes: IndexStorage,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum BlockStorage {
    Local(LocalBlockStorage),
    S3(S3BlockStorage),
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct LocalBlockStorage {
    pub path: PathBuf,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct S3BlockStorage {
    pub bucket: String,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub endpoint: Option<Uri>,
    pub dynamodb: DynamoStorage,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum IndexStorage {
    Local(LocalIndexStorage),
    DynamoDB(DynamoStorage),
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct LocalIndexStorage {
    pub path: PathBuf,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct DynamoStorage {
    pub table: String,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub endpoint: Option<Uri>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Relay {
    pub address: String,
    pub port: u16,
}

impl Default for BlockStorage {
    fn default() -> BlockStorage {
        BlockStorage::Local(LocalBlockStorage::default())
    }
}

impl Default for LocalBlockStorage {
    fn default() -> LocalBlockStorage {
        LocalBlockStorage {
            path: PathBuf::from(r"/tmp/kepler/blocks"),
        }
    }
}

impl Default for IndexStorage {
    fn default() -> IndexStorage {
        IndexStorage::Local(LocalIndexStorage::default())
    }
}

impl Default for LocalIndexStorage {
    fn default() -> LocalIndexStorage {
        LocalIndexStorage {
            path: PathBuf::from(r"/tmp/kepler/indexes"),
        }
    }
}

impl Default for Relay {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".into(),
            port: 8081,
        }
    }
}
