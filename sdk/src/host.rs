use http::uri::Authority;
use lib::resource::{OrbitId, ResourceId};
use lib::{
    cacao_zcap::CacaoToZcapError,
    cacaos::{BasicSignature, SIWESignature},
    siwe::{generate_nonce, Message, TimeStamp, Version},
};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::{util::siwe_to_zcap, zcap::DelegationHeaders};

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
export type HostConfig = {
  address: string,
  chainId: number,
  domain: string,
  issuedAt: string,
  orbitId: string,
  peerId: string,
}
"#;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedMessage {
    #[serde(with = "crate::serde_siwe::message")]
    siwe: Message,
    #[serde(with = "crate::serde_siwe::signature")]
    signature: BasicSignature<SIWESignature>,
}

impl TryFrom<HostConfig> for Message {
    type Error = String;
    fn try_from(c: HostConfig) -> Result<Self, String> {
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
            resources: vec![
                ResourceId::new(c.orbit_id, None, None, Some(String::from("host")))
                    .to_string()
                    .try_into()
                    .map_err(|e| format!("failed to parse orbit id as a URI: {}", e))?,
            ],
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

pub fn host(signed_message: SignedMessage) -> Result<DelegationHeaders, Error> {
    siwe_to_zcap(signed_message.siwe, signed_message.signature)
        .map(DelegationHeaders::new)
        .map_err(Error::UnableToConstructDelegation)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unable to generate the SIWE message: {0}")]
    UnableToGenerateSIWEMessage(String),
    #[error("unable to construct delegation: {0}")]
    UnableToConstructDelegation(CacaoToZcapError),
    #[error("failed to translate response to JSON: {0}")]
    JSONSerializing(serde_json::Error),
    #[error("failed to parse input from JSON: {0}")]
    JSONDeserializing(serde_json::Error),
}
