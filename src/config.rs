use crate::{
    allow_list::OrbitAllowListService,
    storage::{file_system::FileSystemConfig, s3::S3BlockConfig},
    BlockConfig, BlockStage,
};
use kepler_core::keys::StaticSecret;
use rocket::data::ByteUnit;
use serde::{Deserialize, Serialize};
use serde_with::{
    base64::{Base64, UrlSafe},
    formats::Unpadded,
    serde_as, FromInto,
};

#[derive(Serialize, Deserialize, Debug, Default, Clone, Hash, PartialEq, Eq)]
pub struct Config {
    pub log: Logging,
    pub storage: Storage,
    pub orbits: OrbitsConfig,
    pub relay: Relay,
    pub prometheus: Prometheus,
    pub cors: bool,
    pub keys: Keys,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum Keys {
    Static(Static),
}

impl Default for Keys {
    fn default() -> Self {
        Self::Static(Static::default())
    }
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, Default)]
pub struct Static {
    #[serde_as(as = "Option<Base64<UrlSafe, Unpadded>>")]
    secret: Option<Vec<u8>>,
}

#[derive(Debug, thiserror::Error)]
pub enum SecretInitError {
    #[error("Secret required to be at least 32 bytes, but was {0}")]
    NotEnoughEntropy(usize),
    #[error("Missing secret")]
    MissingSecret,
}

impl TryFrom<Static> for StaticSecret {
    type Error = SecretInitError;
    fn try_from(s: Static) -> Result<Self, Self::Error> {
        let secret = s.secret.ok_or(SecretInitError::MissingSecret)?;
        StaticSecret::new(secret).map_err(|v| SecretInitError::NotEnoughEntropy(v.len()))
    }
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
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Storage {
    #[serde_as(as = "FromInto<BlockStorage>")]
    #[serde(default = "fs_store")]
    pub blocks: BlockConfig,
    #[serde_as(as = "FromInto<StagingStorage>")]
    #[serde(default = "memory_stage")]
    pub staging: BlockStage,
    #[serde(default = "memory_db")]
    pub database: String,
    pub limit: Option<ByteUnit>,
}

impl Default for Storage {
    fn default() -> Self {
        Self {
            blocks: BlockStorage::default().into(),
            staging: StagingStorage::default().into(),
            database: memory_db(),
            limit: None,
        }
    }
}

fn memory_db() -> String {
    "sqlite::memory:".to_string()
}

fn memory_stage() -> BlockStage {
    StagingStorage::Memory.into()
}

fn fs_store() -> BlockConfig {
    BlockStorage::default().into()
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum BlockStorage {
    Local(FileSystemConfig),
    S3(S3BlockConfig),
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq, Default)]
pub enum StagingStorage {
    FileSystem,
    #[default]
    Memory,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Relay {
    pub address: String,
    pub port: u16,
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

impl Default for Relay {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".into(),
            port: 8081,
        }
    }
}

impl Default for Prometheus {
    fn default() -> Self {
        Self { port: 8001 }
    }
}
