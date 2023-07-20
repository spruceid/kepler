use http::uri::Authority;
use kepler_lib::authorization::KeplerDelegation;
use kepler_lib::cacaos::{
    siwe::{generate_nonce, Message, TimeStamp, Version},
    siwe_cacao::{SIWESignature, SiweCacao},
};
use kepler_lib::resource::OrbitId;
use kepler_lib::siwe_recap::Builder;
use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr};

use crate::authorization::DelegationHeaders;

#[serde_as]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostConfig {
    #[serde(with = "crate::serde_siwe::address")]
    pub address: [u8; 20],
    pub chain_id: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub domain: Authority,
    #[serde_as(as = "DisplayFromStr")]
    pub issued_at: TimeStamp,
    pub orbit_id: OrbitId,
    pub peer_id: String,
}

#[serde_as]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedMessage {
    #[serde_as(as = "DisplayFromStr")]
    pub siwe: Message,
    #[serde(with = "crate::serde_siwe::signature")]
    pub signature: SIWESignature,
}

impl TryFrom<HostConfig> for Message {
    type Error = String;
    fn try_from(c: HostConfig) -> Result<Self, String> {
        Builder::new()
            .with_action(
                &"kepler"
                    .parse()
                    .map_err(|e| format!("failed to parse kepler as namespace: {e}"))?,
                c.orbit_id.to_resource(None, None, None).to_string(),
                "host".to_string(),
            )
            .build(Self {
                address: c.address,
                chain_id: c.chain_id,
                domain: c.domain,
                issued_at: c.issued_at,
                uri: c
                    .peer_id
                    .try_into()
                    .map_err(|e| format!("error parsing peer as a URI: {e}"))?,
                nonce: generate_nonce(),
                statement: None,
                resources: vec![],
                version: Version::V1,
                not_before: None,
                expiration_time: None,
                request_id: None,
            })
            .map_err(|e| format!("error building Host SIWE message: {e}"))
    }
}

pub fn generate_host_siwe_message(config: HostConfig) -> Result<Message, Error> {
    Message::try_from(config).map_err(Error::UnableToGenerateSIWEMessage)
}

pub fn siwe_to_delegation_headers(signed_message: SignedMessage) -> DelegationHeaders {
    DelegationHeaders::new(KeplerDelegation::Cacao(Box::new(SiweCacao::new(
        signed_message.siwe.into(),
        signed_message.signature,
        None,
    ))))
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
