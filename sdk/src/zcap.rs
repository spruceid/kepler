use lib::zcap::{KeplerDelegation, KeplerInvocation};
use serde::{Deserialize, Serialize};

use crate::session::Session;

#[derive(Debug, Deserialize, Serialize)]
pub struct DelegationHeaders {
    #[serde(with = "serde_b64", rename = "Authorization")]
    delegation: KeplerDelegation,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct InvocationHeaders {
    #[serde(with = "serde_b64", rename = "Authorization")]
    invocation: KeplerInvocation,
}

impl InvocationHeaders {
    pub async fn from(session: Session, path: String, action: String) -> Result<Self, Error> {
        Ok(Self {
            invocation: session
                .invoke(path, action)
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
    FailedToMakeInvocation(lib::zcap::Error),
    #[error("failed to translate response to JSON: {0}")]
    JSONSerializing(serde_json::Error),
    #[error("failed to parse session from JSON: {0}")]
    JSONDeserializing(serde_json::Error),
}

mod serde_b64 {
    use base64::{decode_config, encode_config, URL_SAFE};
    use serde::{
        de::{DeserializeOwned, Error as DeError},
        ser::Error as SerError,
        Deserialize, Deserializer, Serialize, Serializer,
    };

    pub fn deserialize<'de, T, D>(d: D) -> Result<T, D::Error>
    where
        T: DeserializeOwned,
        D: Deserializer<'de>,
    {
        String::deserialize(d)
            .and_then(|encoded| decode_config(encoded, URL_SAFE).map_err(D::Error::custom))
            .and_then(from_json_bytes)
    }

    pub fn serialize<T, S>(t: &T, s: S) -> Result<S::Ok, S::Error>
    where
        T: Serialize,
        S: Serializer,
    {
        serde_json::to_string(t)
            .map_err(S::Error::custom)
            .map(|json_string| encode_config(json_string, URL_SAFE))
            .and_then(|encoded| encoded.serialize(s))
    }

    fn from_json_bytes<T, E>(bytes: Vec<u8>) -> Result<T, E>
    where
        T: DeserializeOwned,
        E: serde::de::Error,
    {
        serde_json::from_slice(&bytes).map_err(E::custom)
    }
}
