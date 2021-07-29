use crate::allow_list::OrbitAllowListService;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Config {
    pub database: Database,
    pub orbit_allow_list: OrbitAllowListService,
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
