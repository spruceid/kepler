use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Config {
    pub database: Database,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Database {
    pub path: PathBuf,
}

impl Default for Database {
    fn default() -> Database {
        Database {
            path: PathBuf::from(r"/tmp"),
        }
    }
}
