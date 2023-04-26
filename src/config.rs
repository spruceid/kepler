use crate::{
    allow_list::OrbitAllowListService,
    storage::{file_system::FileSystemConfig, s3::S3BlockConfig},
    BlockConfig,
};
use libp2p::{build_multiaddr, Multiaddr};
use rocket::http::hyper::Uri;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr, FromInto};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, PartialEq, Eq)]
pub struct Config {
    pub log: Logging,
    pub storage: Storage,
    pub orbits: OrbitsConfig,
    pub relay: Relay,
    pub prometheus: Prometheus,
    pub cors: bool,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, PartialEq, Eq)]
pub struct Logging {
    pub format: LoggingFormat,
    pub tracing: Tracing,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, PartialEq, Eq)]
pub enum LoggingFormat {
    #[default]
    Text,
    Json,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Tracing {
    pub traceheader: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, PartialEq, Eq)]
pub struct OrbitsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowlist: Option<OrbitAllowListService>,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, PartialEq, Eq)]
pub struct Storage {
    #[serde_as(as = "FromInto<BlockStorage>")]
    pub blocks: BlockConfig,
    pub indexes: IndexStorage,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum BlockStorage {
    Local(FileSystemConfig),
    S3(S3BlockConfig),
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
    pub address: Multiaddr,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Prometheus {
    pub port: u16,
}

impl Default for Tracing {
    fn default() -> Tracing {
        Tracing {
            enabled: false,
            traceheader: "Spruce-Trace-Id".to_string(),
        }
    }
}

impl Default for BlockStorage {
    fn default() -> BlockStorage {
        BlockStorage::Local(FileSystemConfig::default())
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
            address: build_multiaddr!(Ip4([127, 0, 0, 1]), Tcp(8081u16)),
        }
    }
}

impl Default for Prometheus {
    fn default() -> Self {
        Self { port: 8001 }
    }
}
