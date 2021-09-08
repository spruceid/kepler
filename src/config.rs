use crate::allow_list::OrbitAllowListService;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Config {
    pub database: Database,
    pub orbits: OrbitsConfig,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct OrbitsConfig {
    #[serde(default)]
    pub public: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowlist: Option<OrbitAllowListService>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Database {
    pub path: PathBuf,
}

impl Default for Database {
    fn default() -> Database {
        Database {
            path: PathBuf::from(r"/tmp/kepler"),
        }
    }
}
