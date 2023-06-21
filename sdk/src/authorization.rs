use kepler_lib::authorization::{KeplerDelegation, KeplerInvocation};
use serde::{Deserialize, Serialize};

use crate::session::Session;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DelegationHeaders {
    #[serde(with = "header_enc", rename = "Authorization")]
    delegation: KeplerDelegation,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct InvocationHeaders {
    #[serde(with = "header_enc", rename = "Authorization")]
    invocation: KeplerInvocation,
}

impl InvocationHeaders {
    pub async fn from(
        session: Session,
        actions: Vec<(String, String, String)>,
    ) -> Result<Self, Error> {
        Ok(Self {
            invocation: session
                .invoke(actions)
                .await
                .map_err(Error::FailedToMakeInvocation)?,
        })
    }
}

impl DelegationHeaders {
    pub fn new(delegation: KeplerDelegation) -> Self {
        Self { delegation }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to generate proof for invocation: {0}")]
    FailedToMakeInvocation(kepler_lib::authorization::InvocationError),
    #[error("failed to translate response to JSON: {0}")]
    JSONSerializing(serde_json::Error),
    #[error("failed to parse session from JSON: {0}")]
    JSONDeserializing(serde_json::Error),
}

mod header_enc {
    use kepler_lib::authorization::HeaderEncode;
    use serde::{
        de::Error as DeError, ser::Error as SerError, Deserialize, Deserializer, Serialize,
        Serializer,
    };

    pub fn deserialize<'de, T, D>(d: D) -> Result<T, D::Error>
    where
        T: HeaderEncode,
        D: Deserializer<'de>,
    {
        String::deserialize(d)
            .and_then(|encoded| T::decode(&encoded).map_err(D::Error::custom).map(|t| t.0))
    }

    pub fn serialize<T, S>(t: &T, s: S) -> Result<S::Ok, S::Error>
    where
        T: HeaderEncode,
        S: Serializer,
    {
        t.encode()
            .map_err(S::Error::custom)
            .and_then(|encoded| encoded.serialize(s))
    }
}
