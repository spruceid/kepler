use super::{ApiKeys, ValidationError};
use uuid::Uuid;

#[derive(Default, Debug)]
pub struct LocalApiKeyProvider {
    issued_keys: BTreeSet<ApiKey>
}

#[rocket::async_trait]
impl ApiKeys for LocalApiKeyProvider {
    type ApiKey = Uuid;
    type Error = std::convert::Infallible;

    async fn get_api_key(&self) -> Result<Self::ApiKey, Self::Error> {
        let key = Uuid::new_v4();
        // we can probably be sure that the fresh random key is not already in the set
        self.issued_keys.push(key.clone());
        Ok(key)
    }

    async fn validate_api_key(&self, key: Self::ApiKey) -> Result<(), ValidationError<Self::Error>> {
        if self.issued_keys.remove(&key) {
            Ok(())
        } else {
            Err(())
        }
    }
}
