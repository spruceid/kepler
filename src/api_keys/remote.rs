use super::{ApiKeys, ValidationError};
use reqwest::{Client, Error, Url};

#[derive(Debug, Clone)]
pub struct RemoteApiKeyProvider {
    url: Url,
    client: Client
}

impl RemoteApiKeyProvider {
    pub fn new(url: Url) -> Self {
        Self { url }
    }
}

#[rocket::async_trait]
impl ApiKeys for RemoteApiKeyProvider {
    type ApiKey = String;
    type Error = Error;

    async fn get_api_key(&self) -> Result<Self::ApiKey, Self::Error> {
        Ok(self.client.get(&self.url).send().await?.error_for_status()?.text().await?)
    }

    async fn validate_api_key(&self, api_key: Self::ApiKey) -> Result<(), ValidationError<Self::Error>> {
        self.client.post(&self.url).body(api_key).send().await?.error_for_status()?;
        Ok(())
    }
}
