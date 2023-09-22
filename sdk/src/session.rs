use crate::authorization::DelegationHeaders;
use http::uri::Authority;
use kepler_lib::{
    authorization::{Delegation, Invocation},
    cacaos::siwe::{generate_nonce, Message, TimeStamp, Version as SIWEVersion},
    libipld::Cid,
    resolver::DID_METHODS,
    resource::OrbitId,
    ssi::{did::Source, jwk::JWK, ucan::capabilities::Capabilities, vc::get_verification_method},
};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use time::OffsetDateTime;

pub struct SIWESignature(pub [u8; 65]);

#[serde_as]
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    #[serde(default)]
    pub actions: Capabilities<serde_json::Value>,
    #[serde(with = "crate::serde_siwe::address")]
    pub address: [u8; 20],
    pub chain_id: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub domain: Authority,
    #[serde_as(as = "DisplayFromStr")]
    pub issued_at: TimeStamp,
    pub orbit_id: OrbitId,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub not_before: Option<TimeStamp>,
    #[serde_as(as = "DisplayFromStr")]
    pub expiration_time: TimeStamp,
    #[serde_as(as = "Option<Vec<DisplayFromStr>>")]
    #[serde(default)]
    pub parents: Option<Vec<Cid>>,
    #[serde(default)]
    pub jwk: Option<JWK>,
}

#[serde_as]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PreparedSession {
    pub jwk: JWK,
    pub orbit_id: OrbitId,
    #[serde_as(as = "DisplayFromStr")]
    pub siwe: Message,
    pub verification_method: String,
}

#[serde_as]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedSession {
    #[serde(flatten)]
    pub session: PreparedSession,
    #[serde(with = "crate::serde_siwe::signature")]
    pub signature: SIWESignature,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub delegation_header: DelegationHeaders,
    #[serde_as(as = "DisplayFromStr")]
    pub delegation_cid: Cid,
    pub jwk: JWK,
    pub orbit_id: OrbitId,
    pub verification_method: String,
}

impl SessionConfig {
    fn into_message(self, delegate: &str) -> Result<Message, String> {
        use kepler_lib::siwe_recap::Capability;
        let caps =
            self.actions
                .into_inner()
                .into_iter()
                .fold(Capability::new(), |mut caps, (k, v)| {
                    caps.with_actions(k, v);
                    caps
                });
        match self.parents {
            Some(p) => caps.with_proofs(&p),
            None => caps,
        }
        .build_message(Message {
            address: self.address,
            chain_id: self.chain_id,
            domain: self.domain,
            expiration_time: Some(self.expiration_time),
            issued_at: self.issued_at,
            nonce: generate_nonce(),
            not_before: self.not_before,
            request_id: None,
            statement: None,
            resources: vec![],
            uri: delegate
                .try_into()
                .map_err(|e| format!("failed to parse session key DID as a URI: {e}"))?,
            version: SIWEVersion::V1,
        })
        .map_err(|e| format!("error building Host SIWE message: {e}"))
    }
}

impl Session {
    pub fn invoke(self, actions: Vec<(String, String, String)>) -> Result<Invocation, Error> {
        use kepler_lib::ssi::ucan::Payload;
        use serde_json::Value;
        let targets =
            actions
                .into_iter()
                .try_fold(Capabilities::<Value>::new(), |mut acc, (s, p, a)| {
                    let action = format!("{s}/{a}");
                    acc.with_action_convert(
                        self.orbit_id
                            .clone()
                            .to_resource(Some(s), Some(p), None)
                            .to_string(),
                        action,
                        [],
                    )
                    .map_err(|_| Error::InvalidAction)?;
                    Ok::<Capabilities<Value>, Error>(acc)
                })?;
        let now = OffsetDateTime::now_utc();
        let unix = now.unix_timestamp();
        // 60 seconds in the future
        let exp = unix + 60;
        let mut p = Payload::<Value, Value>::new(
            self.verification_method.clone(),
            self.verification_method,
        );
        p.expiration = Some(exp as u64);
        p.proof = Some(vec![self.delegation_cid]);
        p.nonce = Some(format!("urn:uuid:{}", uuid::Uuid::new_v4()));
        p.capabilities = targets;
        Ok(p.sign_with_jwk(&self.jwk, None)?.try_into()?)
    }
}

pub async fn prepare_session(config: SessionConfig) -> Result<PreparedSession, Error> {
    let mut jwk = match &config.jwk {
        Some(k) => k.clone(),
        None => JWK::generate_ed25519()?,
    };
    jwk.algorithm = Some(kepler_lib::ssi::jwk::Algorithm::EdDSA);

    let did = DID_METHODS
        .generate(&Source::KeyAndPattern(&jwk, "key"))
        .ok_or(Error::UnableToGenerateDID)?;
    let did_resolver = DID_METHODS.to_resolver();
    let verification_method = get_verification_method(&did, did_resolver)
        .await
        .ok_or(Error::UnableToGenerateDID)?;

    let orbit_id = config.orbit_id.clone();

    let siwe = config
        .into_message(&verification_method)
        .map_err(Error::UnableToGenerateSIWEMessage)?;

    Ok(PreparedSession {
        orbit_id,
        jwk,
        verification_method,
        siwe,
    })
}

pub fn complete_session_setup(signed_session: SignedSession) -> Result<Session, Error> {
    use kepler_lib::libipld::multihash::{Code, MultihashDigest};
    let delegation =
        Delegation::try_from((signed_session.session.siwe, signed_session.signature.0))?;
    let delegation_cid = Cid::new_v1(
        0x71,
        Code::Blake3_256.digest(&serde_ipld_dagcbor::to_vec(&delegation)?),
    );
    let delegation_header = DelegationHeaders::new(delegation);

    Ok(Session {
        delegation_header,
        delegation_cid,
        jwk: signed_session.session.jwk,
        orbit_id: signed_session.session.orbit_id,
        verification_method: signed_session.session.verification_method,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unable to generate session key: {0}")]
    UnableToGenerateKey(#[from] kepler_lib::ssi::jwk::Error),
    #[error("unable to generate the DID of the session key")]
    UnableToGenerateDID,
    #[error("unable to generate the SIWE message to start the session: {0}")]
    UnableToGenerateSIWEMessage(String),
    #[error("unable to generate the CID: {0}")]
    UnableToGenerateCid(#[from] serde_ipld_dagcbor::EncodeError<std::collections::TryReserveError>),
    #[error("failed to translate response to JSON: {0}")]
    JSONSerializing(serde_json::Error),
    #[error("failed to parse input from JSON: {0}")]
    JSONDeserializing(serde_json::Error),
    #[error(transparent)]
    CacaoError(#[from] kepler_lib::cacaos::common::Error),
    #[error(transparent)]
    UcanError(#[from] kepler_lib::ssi::ucan::payload::Error),
    #[error("Invalid Actions or Resources")]
    InvalidAction,
}

#[cfg(test)]
pub mod test {
    use super::*;
    use serde_json::json;
    pub async fn test_session() -> Session {
        let config = json!({
            "actions": {
                "kepler:pkh:eip155:1:0x7BD63AA37326a64d458559F44432103e3d6eEDE9://default/kv/path": {
                    "kv/put": [{}],
                    "kv/get": [{}],
                    "kv/list": [{}],
                    "kv/del": [{}],
                    "kv/metadata": [{}],
                },
            },
            "address": "0x7BD63AA37326a64d458559F44432103e3d6eEDE9",
            "chainId": 1u8,
            "domain": "example.com",
            "issuedAt": "2022-01-01T00:00:00.000Z",
            "orbitId": "kepler:pkh:eip155:1:0x7BD63AA37326a64d458559F44432103e3d6eEDE9://default",
            "expirationTime": "3000-01-01T00:00:00.000Z",
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
            .invoke(vec![("kv".into(), "path".into(), "get".into())])
            .expect("failed to create invocation");
    }
}
