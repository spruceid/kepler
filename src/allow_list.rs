use anyhow::Result;
use libipld::{cid::Cid, multibase::Base};
use reqwest::get;
use serde::{Deserialize, Serialize};
use ssi::did::DIDURL;

#[rocket::async_trait]
pub trait OrbitAllowList {
    async fn is_allowed(&self, oid: &Cid) -> Result<Vec<DIDURL>>;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(from = "String", into = "String")]
pub struct OrbitAllowListService(pub String);

impl From<String> for OrbitAllowListService {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<OrbitAllowListService> for String {
    fn from(oals: OrbitAllowListService) -> Self {
        oals.0
    }
}

#[rocket::async_trait]
impl OrbitAllowList for OrbitAllowListService {
    async fn is_allowed(&self, oid: &Cid) -> Result<Vec<DIDURL>> {
        Ok(
            get([self.0.as_str(), &oid.to_string_of_base(Base::Base58Btc)?].join("/"))
                .await?
                .error_for_status()?
                .json::<Vec<DIDURL>>()
                .await?,
        )
    }
}
