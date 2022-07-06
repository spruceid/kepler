use http::uri::Authority;
use iri_string::types::UriString;
use kepler_lib::cacaos::{
    siwe::{nonce::generate_nonce, Message, TimeStamp, Version},
    siwe_cacao::{SIWESignature, SiweCacao},
};
use kepler_lib::resource::OrbitId;
use kepler_lib::zcap::KeplerDelegation;
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::zcap::DelegationHeaders;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostConfig {
    #[serde(with = "crate::serde_siwe::address")]
    address: [u8; 20],
    chain_id: u64,
    #[serde(with = "crate::serde_siwe::domain")]
    domain: Authority,
    #[serde(with = "crate::serde_siwe::timestamp")]
    issued_at: TimeStamp,
    orbit_id: OrbitId,
    peer_id: String,
}

#[wasm_bindgen(typescript_custom_section)]
const TS_DEF: &'static str = r#"
/**
 * Configuration object for generating a Orbit Host Delegation SIWE message.
 */
export type HostConfig = {
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
  /** The peer that is the target/invoker in the delegation. */
  peerId: string,
}
"#;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedMessage {
    #[serde(with = "crate::serde_siwe::message")]
    siwe: Message,
    #[serde(with = "crate::serde_siwe::signature")]
    signature: SIWESignature,
}

impl TryFrom<HostConfig> for Message {
    type Error = String;
    fn try_from(c: HostConfig) -> Result<Self, String> {
        let root_cap: UriString = c
            .orbit_id
            .to_string()
            .try_into()
            .map_err(|e| format!("failed to parse orbit id as a URI: {}", e))?;
        Ok(Self {
            address: c.address,
            chain_id: c.chain_id,
            domain: c.domain,
            issued_at: c.issued_at,
            uri: format!("peer:{}", c.peer_id)
                .try_into()
                .map_err(|e| format!("error parsing peer as a URI: {}", e))?,
            nonce: generate_nonce(),
            statement: Some(
                "Authorize action (host): Authorize this peer to host your orbit.".into(),
            ),
            resources: vec![root_cap.clone(), root_cap],
            version: Version::V1,
            not_before: None,
            expiration_time: None,
            request_id: None,
        })
    }
}

pub fn generate_host_siwe_message(config: HostConfig) -> Result<Message, Error> {
    Message::try_from(config).map_err(Error::UnableToGenerateSIWEMessage)
}

pub fn host(signed_message: SignedMessage) -> DelegationHeaders {
    DelegationHeaders::new(KeplerDelegation::Cacao(SiweCacao::new(
        signed_message.siwe.into(),
        signed_message.signature,
        None,
    )))
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unable to generate the SIWE message: {0}")]
    UnableToGenerateSIWEMessage(String),
    #[error("failed to translate response to JSON: {0}")]
    JSONSerializing(serde_json::Error),
    #[error("failed to parse input from JSON: {0}")]
    JSONDeserializing(serde_json::Error),
}
