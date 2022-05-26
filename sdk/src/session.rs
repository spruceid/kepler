use http::uri::Authority;
use lib::{
    didkit::DID_METHODS,
    resource::OrbitId,
    ssi::{
        cacao_zcap::{
            cacaos::{
                siwe::{nonce::generate_nonce, Message, TimeStamp, Version as SIWEVersion},
                siwe_cacao::SIWESignature,
                BasicSignature,
            },
            translation::cacao_to_zcap::CacaoToZcapError,
        },
        did::Source,
        jwk::JWK,
        vc::get_verification_method,
    },
    zcap::{make_invocation, Error as ZcapError, KeplerDelegation, KeplerInvocation},
};
use serde::{Deserialize, Serialize};

use crate::util::siwe_to_zcap;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    actions: Vec<String>,
    #[serde(with = "crate::serde_siwe::address")]
    address: [u8; 20],
    chain_id: u64,
    #[serde(with = "crate::serde_siwe::domain")]
    domain: Authority,
    #[serde(with = "crate::serde_siwe::timestamp")]
    issued_at: TimeStamp,
    orbit_id: OrbitId,
    #[serde(default, with = "crate::serde_siwe::optional_timestamp")]
    not_before: Option<TimeStamp>,
    #[serde(with = "crate::serde_siwe::timestamp")]
    expiration_time: TimeStamp,
    service: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedSession {
    jwk: JWK,
    orbit_id: OrbitId,
    service: String,
    #[serde(with = "crate::serde_siwe::message")]
    siwe: Message,
    verification_method: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedSession {
    jwk: JWK,
    orbit_id: OrbitId,
    service: String,
    #[serde(with = "crate::serde_siwe::signature")]
    signature: BasicSignature<SIWESignature>,
    #[serde(with = "crate::serde_siwe::message")]
    siwe: Message,
    verification_method: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    delegation: KeplerDelegation,
    jwk: JWK,
    orbit_id: OrbitId,
    service: String,
    verification_method: String,
}

impl SessionConfig {
    fn to_message(self, delegate: &str) -> Result<Message, String> {
        let root_cap = self
            .orbit_id
            .to_string()
            .try_into()
            .map_err(|e| format!("failed to parse orbit id as a URI: {}", e))?;
        let invocation_target = self
            .orbit_id
            .to_resource(Some(self.service), None, None)
            .to_string()
            .try_into()
            .map_err(|e| format!("failed to parse invocation target as a URI: {}", e))?;
        Ok(Message {
            address: self.address,
            chain_id: self.chain_id,
            domain: self.domain,
            expiration_time: Some(self.expiration_time),
            issued_at: self.issued_at,
            nonce: generate_nonce(),
            not_before: self.not_before,
            request_id: None,
            resources: vec![invocation_target, root_cap],
            statement: Some(format!(
                "Authorize action{}: Allow access to your Kepler orbit using this session key.",
                {
                    if self.actions.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", self.actions.join(", "))
                    }
                }
            )),
            uri: delegate
                .try_into()
                .map_err(|e| format!("failed to parse session key DID as a URI: {}", e))?,
            version: SIWEVersion::V1,
        })
    }
}

impl Session {
    pub async fn invoke(self, path: String, action: String) -> Result<KeplerInvocation, ZcapError> {
        let target = self
            .orbit_id
            .to_resource(Some(self.service), Some(path), Some(action));
        make_invocation(target, self.delegation, &self.jwk, self.verification_method).await
    }
}

pub async fn prepare_session(config: SessionConfig) -> Result<PreparedSession, Error> {
    let jwk = JWK::generate_ed25519().map_err(Error::UnableToGenerateKey)?;

    let did = DID_METHODS
        .generate(&Source::KeyAndPattern(&jwk, "key"))
        .ok_or(Error::UnableToGenerateDID)?;
    let did_resolver = DID_METHODS.to_resolver();
    let verification_method = get_verification_method(&did, did_resolver)
        .await
        .ok_or(Error::UnableToGenerateDID)?;

    let orbit_id = config.orbit_id.clone();
    let service = config.service.clone();

    let siwe = config
        .to_message(&verification_method)
        .map_err(Error::UnableToGenerateSIWEMessage)?;

    Ok(PreparedSession {
        orbit_id,
        service,
        jwk,
        verification_method,
        siwe,
    })
}

pub fn complete_session_setup(signed_session: SignedSession) -> Result<Session, Error> {
    Ok(Session {
        delegation: siwe_to_zcap(signed_session.siwe, signed_session.signature)
            .map_err(Error::UnableToConstructDelegation)?,
        jwk: signed_session.jwk,
        orbit_id: signed_session.orbit_id,
        service: signed_session.service,
        verification_method: signed_session.verification_method,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unable to generate session key: {0}")]
    UnableToGenerateKey(lib::ssi::error::Error),
    #[error("unable to generate the DID of the session key")]
    UnableToGenerateDID,
    #[error("unable to generate the SIWE message to start the session: {0}")]
    UnableToGenerateSIWEMessage(String),
    #[error("unable to construct delegation: {0}")]
    UnableToConstructDelegation(CacaoToZcapError),
    #[error("failed to translate response to JSON: {0}")]
    JSONSerializing(serde_json::Error),
    #[error("failed to parse input from JSON: {0}")]
    JSONDeserializing(serde_json::Error),
}

#[cfg(test)]
pub mod test {
    use super::*;
    use serde_json::json;
    pub async fn test_session() -> Session {
        let config = json!({
            "actions": vec!["put", "get", "list", "del", "metadata"],
            "address": "0x7BD63AA37326a64d458559F44432103e3d6eEDE9",
            "chainId": 1u8,
            "domain": "example.com",
            "issuedAt": "2022-01-01T00:00:00.000Z",
            "orbitId": "kepler:pkh:eip155:1:0x7BD63AA37326a64d458559F44432103e3d6eEDE9://default",
            "expirationTime": "3000-01-01T00:00:00.000Z",
            "service": "kv",
        });
        let prepared = prepare_session(serde_json::from_value(config).unwrap())
            .await
            .unwrap();
        let mut signed = serde_json::to_value(prepared).unwrap();
        signed.as_object_mut()
            .unwrap()
            .insert(
                "signature".into(),
                "361647d08fb3ac41b26d9300d80e1964e1b3e7960e5276b3c9f5045ae55171442287279c83fd8922f9238312e89336b1672be8778d078d7dc5107b8c913299721c".into()
            );
        complete_session_setup(serde_json::from_value(signed).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn create_session_and_invoke() {
        &test_session()
            .await
            .invoke("path".into(), "get".into())
            .await
            .expect("failed to create invocation");
    }
}
