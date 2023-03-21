#[derive(thiserror::Error, Debug)]
pub enum ValidationError<E>
where
    E: std::error::Error + 'static,
{
    #[error("Invalid API key")]
    InvalidApiKey,
    #[error(transparent)]
    Other(#[from] E),
}

#[rocket::async_trait]
pub trait ApiKeys {
    type ApiKey;
    type Error: std::error::Error + 'static;
    async fn get_api_key(&self) -> Result<Self::ApiKey, Self::Error>;
    async fn validate_api_key(&self, key: Self::ApiKey)
        -> Result<(), ValidationError<Self::Error>>;
}
