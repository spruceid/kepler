use chrono::Utc;
use http::uri::Authority;
use kepler_lib::{
    cacaos::siwe::{nonce::generate_nonce, Message, TimeStamp, Version as SIWEVersion},
    didkit::DID_METHODS,
    libipld::Cid,
    resource::OrbitId,
    ssi::{did::Source, jwk::JWK, vc::get_verification_method},
    zcap::{make_invocation, InvocationError as ZcapError, KeplerInvocation},
};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use wasm_bindgen::prelude::*;

#[serde_as]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    actions: Vec<String>,
    #[serde(with = "crate::serde_siwe::address")]
    address: [u8; 20],
    chain_id: u64,
    #[serde_as(as = "DisplayFromStr")]
    domain: Authority,
    #[serde_as(as = "DisplayFromStr")]
    issued_at: TimeStamp,
    orbit_id: OrbitId,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    not_before: Option<TimeStamp>,
    #[serde_as(as = "DisplayFromStr")]
    expiration_time: TimeStamp,
    service: String,
}

#[wasm_bindgen(typescript_custom_section)]
const TS_DEF: &'static str = r#"
/**
 * Configuration object for starting a Kepler session.
 */
export type SessionConfig = {
  /** Actions that the session key will be permitted to perform. */
  actions: string[],
  /** Ethereum address. */
  address: string,
  /** Chain ID. */
  chainId: number,
  /** Domain of the webpage. */
  domain: string,
  /** Current time for SIWE message. */
  issuedAt: string,
  /** The orbit that is the target resource of the delegation. */
  orbitId: string,
  /** The earliest time that the session will be valid from. */
  notBefore?: string,
  /** The latest time that the session will be valid until. */
  expirationTime: string,
  /** The service that the session key will be permitted to perform actions against. */
  service: string,
}
"#;

#[serde_as]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedSession {
    jwk: JWK,
    orbit_id: OrbitId,
    service: String,
    #[serde_as(as = "DisplayFromStr")]
    siwe: Message,
    verification_method: String,
}

#[serde_as]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedSession {
    #[serde_as(as = "DisplayFromStr")]
    delegation: Cid,
    jwk: JWK,
    orbit_id: OrbitId,
    service: String,
    verification_method: String,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    #[serde_as(as = "DisplayFromStr")]
    delegation: Cid,
    jwk: JWK,
    orbit_id: OrbitId,
    service: String,
    verification_method: String,
}

#[wasm_bindgen(typescript_custom_section)]
const TS_DEF: &'static str = r#"
/**
 * A Kepler session.
 */
export type Session = {
  /** The delegation reference from the user to the session key. */
  delegation: string,
  /** The session key. */
  jwk: object,
  /** The orbit that the session key is permitted to perform actions against. */
  orbitId: string,
  /** The service that the session key is permitted to perform actions against. */
  service: string,
  /** The verification method of the session key. */
  verificationMethod: string,
}
"#;

impl SessionConfig {
    fn into_message(self, delegate: &str) -> Result<Message, String> {
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
        Ok(make_invocation(
            target,
            self.delegation,
            &self.jwk,
            self.verification_method,
            // 60 seconds in the future
            (Utc::now().timestamp_nanos() as f64 / 1e+9_f64) + 60.0,
            None,
            None,
        )
        .await?)
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
        .into_message(&verification_method)
        .map_err(Error::UnableToGenerateSIWEMessage)?;

    Ok(PreparedSession {
        orbit_id,
        service,
        jwk,
        verification_method,
        siwe,
    })
}

pub fn complete_session_setup(signed_session: SignedSession) -> Session {
    Session {
        delegation: signed_session.delegation,
        jwk: signed_session.jwk,
        orbit_id: signed_session.orbit_id,
        service: signed_session.service,
        verification_method: signed_session.verification_method,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unable to generate session key: {0}")]
    UnableToGenerateKey(kepler_lib::ssi::error::Error),
    #[error("unable to generate the DID of the session key")]
    UnableToGenerateDID,
    #[error("unable to generate the SIWE message to start the session: {0}")]
    UnableToGenerateSIWEMessage(String),
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
        test_session()
            .await
            .invoke("path".into(), "get".into())
            .await
            .expect("failed to create invocation");
    }
}
