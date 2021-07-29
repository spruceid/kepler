use anyhow::Result;
use ipfs_embed::Cid;
use libipld::multibase::Base;
use reqwest::{get, StatusCode};
use serde::{Deserialize, Serialize};

#[rocket::async_trait]
pub trait OrbitAllowList {
    async fn is_allowed(&self, oid: &Cid) -> Result<bool>;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OrbitAllowListService {
    pub api: String,
}

impl Default for OrbitAllowListService {
    fn default() -> Self {
        Self {
            api: "http://localhost:11000".into(),
        }
    }
}

#[rocket::async_trait]
impl OrbitAllowList for OrbitAllowListService {
    async fn is_allowed(&self, oid: &Cid) -> Result<bool> {
        match get([self.api.as_str(), &oid.to_string_of_base(Base::Base58Btc)?].join("/"))
            .await?
            .status()
        {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            s => Err(anyhow!(
                "Invalid allow list service response: {}",
                s.as_str()
            )),
        }
    }
}
