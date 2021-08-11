use crate::allow_list::OrbitAllowListService;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Config {
    pub database: Database,
    pub tzkt: Tzkt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orbit_allow_list: Option<OrbitAllowListService>,
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Tzkt {
    pub api: String,
}

impl Default for Tzkt {
    fn default() -> Self {
        Self {
            api: "http://localhost:5000".into(),
        }
    }
}
