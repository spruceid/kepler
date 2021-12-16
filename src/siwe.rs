use crate::{
    auth::{Action, AuthorizationPolicy, AuthorizationToken},
    orbit::{hash_same, OrbitMetadata},
    zcap::KeplerInvocation,
};
use anyhow::Result;
use didkit::DID_METHODS;
use ipfs_embed::Cid;
use libipld::cid::{
    multibase::Base,
    multihash::{Code, MultihashDigest},
};
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};

use hex::FromHex;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use siwe::eip4361::Message;
use std::str::FromStr;

pub struct SIWESignature([u8; 65]);

impl core::fmt::Display for SIWESignature {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0))
    }
}

impl FromStr for SIWESignature {
    type Err = anyhow::Error;
    fn from_str<'a>(s: &'a str) -> Result<Self, Self::Err> {
        match s.split_once("0x") {
            Some(("", h)) => Ok(Self(<[u8; 65]>::from_hex(h)?)),
            _ => Err(anyhow!("Invalid hex string, no leading 0x")),
        }
    }
}

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct SIWEMessage(
    #[serde_as(as = "DisplayFromStr")] Message,
    #[serde_as(as = "DisplayFromStr")] SIWESignature,
);

pub struct SIWETokens {
    pub invocation: KeplerInvocation,
    pub delegation: SIWEMessage,
}

pub struct SIWECreate {
    pub message: SIWEMessage,
    pub orbit: Cid,
    pub action: Action,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for SIWETokens {
    type Error = anyhow::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match (
            request.headers().get_one("x-kepler-invocation").map(|b64| {
                base64::decode_config(b64, base64::URL_SAFE)
                    .map_err(|e| anyhow!(e))
                    .and_then(|s| serde_json::from_slice(&s).map_err(|e| anyhow!(e)))
            }),
            request.headers().get_one("x-siwe-delegation").map(|b64| {
                base64::decode_config(b64, base64::URL_SAFE)
                    .map_err(|e| anyhow!(e))
                    .and_then(|s| serde_json::from_slice(&s).map_err(|e| anyhow!(e)))
            }),
        ) {
            (Some(Ok(invocation)), Some(Ok(delegation))) => Outcome::Success(Self {
                invocation,
                delegation,
            }),
            (Some(Err(e)), _) => {
                tracing::debug!("{}", e);
                Outcome::Failure((Status::Unauthorized, e))
            }
            (_, Some(Err(e))) => {
                tracing::debug!("{}", e);
                Outcome::Failure((Status::Unauthorized, e))
            }
            (_, _) => Outcome::Forward(()),
        }
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for SIWECreate {
    type Error = anyhow::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match request.headers().get_one("x-siwe-invocation").map(|b64| {
            base64::decode_config(b64, base64::URL_SAFE)
                .map_err(|e| anyhow!(e))
                .and_then(|s| Ok(serde_json::from_slice(&s)?))
                .and_then(|message: SIWEMessage| {
                    let params = &message
                        .0
                        .uri
                        .as_str()
                        .split_once("kepler://")
                        .and_then(|u| match u {
                            ("", p) => Some(p),
                            _ => None,
                        })
                        .ok_or_else(|| anyhow!("Invalid URI"))?;

                    let oid: Cid = match params.parse() {
                        Ok(cid) => cid,
                        Err(_) => Cid::new_v1(0x55, Code::Blake2b256.digest(params.as_bytes())),
                    };
                    let host_permission =
                        format!("kepler://{}#host", oid.to_string_of_base(Base::Base58Btc)?);

                    Ok(SIWECreate {
                        orbit: oid,
                        action: match &message.0.resources.first().map(|u| u.as_str()) {
                            Some(uri) if *uri == host_permission => Ok(Action::Create {
                                parameters: params.to_string(),
                                content: vec![],
                            }),
                            _ => Err(anyhow!("Incorrect resources")),
                        }?,
                        message,
                    })
                })
        }) {
            Some(Ok(invocation)) => Outcome::Success(invocation),
            Some(Err(e)) => {
                tracing::debug!("{}", e);
                Outcome::Failure((Status::Unauthorized, e))
            }
            None => Outcome::Forward(()),
        }
    }
}

impl AuthorizationToken for SIWETokens {
    fn action(&self) -> &Action {
        &self.invocation.property_set.capability_action
    }
    fn target_orbit(&self) -> &Cid {
        &self.invocation.property_set.invocation_target
    }
}

impl AuthorizationToken for SIWECreate {
    fn action(&self) -> &Action {
        &self.action
    }
    fn target_orbit(&self) -> &Cid {
        &self.orbit
    }
}

#[rocket::async_trait]
impl AuthorizationPolicy<SIWETokens> for OrbitMetadata {
    async fn authorize(&self, auth_token: &SIWETokens) -> Result<()> {
        // check delegator is controller
        if !self.controllers().contains(
            &format!(
                "did:pkh:eip155:{}:0x{}#blockchainAccountId",
                &auth_token.delegation.0.chain_id,
                &hex::encode(auth_token.delegation.0.address)
            )
            .parse()?,
        ) {
            return Err(anyhow!("Delegator not authorized"));
        };

        let invoker = auth_token
            .invocation
            .proof
            .as_ref()
            .and_then(|proof| proof.verification_method.as_ref())
            .ok_or_else(|| anyhow!("Missing invoker verification method"))?;

        // check delegation to invoker
        if auth_token.delegation.0.uri.as_str() != invoker {
            return Err(anyhow!("Invoker not authorized"));
        };

        // check invoker invokes delegation
        if &format!("urn:siwe:kepler:{}", auth_token.delegation.0.nonce)
            != &auth_token
                .invocation
                .proof
                .as_ref()
                .and_then(|p| p.property_set.as_ref())
                .and_then(|s| s.get("capability"))
                .and_then(|c| c.as_str())
                .ok_or_else(|| anyhow!("Invalid capability in invocation proof"))?
        {
            return Err(anyhow!("Invocation is not for given Delegation"));
        };

        // check delegation time validity
        if !auth_token.delegation.0.valid_now() {
            return Err(anyhow!("Delegation has Expired"));
        };

        // check action is authorized by blanket root ("." as relative path against "kepler://<orbit-id>/") auth
        if !auth_token.delegation.0.resources.contains(&match auth_token
            .invocation
            .property_set
            .capability_action
        {
            Action::List => format!(
                "kepler://{}#list",
                &auth_token.invocation.property_set.invocation_target
            )
            .parse()?,
            Action::Put(_) => format!(
                "kepler://{}#put",
                &auth_token.invocation.property_set.invocation_target
            )
            .parse()?,
            Action::Get(_) => format!(
                "kepler://{}#get",
                &auth_token.invocation.property_set.invocation_target
            )
            .parse()?,
            Action::Del(_) => format!(
                "kepler://{}#del",
                &auth_token.invocation.property_set.invocation_target
            )
            .parse()?,
            _ => return Err(anyhow!("Invalid Action")),
        }) {
            return Err(anyhow!("Invoked action not authorized by delegation"));
        };

        auth_token
            .delegation
            .0
            .verify_eip191(auth_token.delegation.1 .0)?;

        match auth_token
            .invocation
            .verify_signature(Default::default(), DID_METHODS.to_resolver())
            .await
            .errors
            .pop()
        {
            Some(e) => Err(anyhow!(e)),
            None => Ok(()),
        }
    }
}

#[rocket::async_trait]
impl AuthorizationPolicy<SIWEMessage> for OrbitMetadata {
    async fn authorize(&self, auth_token: &SIWEMessage) -> Result<()> {
        // check address is controller
        if !self.controllers().contains(
            &format!(
                "did:pkh:eip155:{}:0x{}#blockchainAccountId",
                &auth_token.0.chain_id,
                &hex::encode(auth_token.0.address)
            )
            .parse()?,
        ) {
            return Err(anyhow!("Delegator not authorized"));
        };
        // check orbit ID
        if self.id()
            != &auth_token
                .0
                .uri
                .as_str()
                .split_once("kepler://")
                .ok_or_else(|| anyhow!("Invalid URI"))
                .and_then(|(_, p)| match Cid::from_str(p) {
                    Ok(oid) => Ok(oid),
                    Err(e) => hash_same(self.id(), p),
                })?
        {
            return Err(anyhow!("Incorrect Orbit ID"));
        };
        // check time validity
        if !auth_token.0.valid_now() {
            return Err(anyhow!("Message has Expired"));
        };

        auth_token.0.verify_eip191(auth_token.1 .0)?;
        Ok(())
    }
}

#[test]
async fn basic() -> Result<()> {
    let d = r#"["localhost wants you to sign in with your Ethereum account:\n0xA391f7adD776806c4dFf3886BBe6370be8F73683\n\nAllow localhost to access your orbit using their temporary session key: did:key:z6MksaFv5D1zYGCvDt2fEvDQWhVcMcaSieMmCSc54DDq3Rwh#z6MksaFv5D1zYGCvDt2fEvDQWhVcMcaSieMmCSc54DDq3Rwh\n\nURI: did:key:z6MksaFv5D1zYGCvDt2fEvDQWhVcMcaSieMmCSc54DDq3Rwh#z6MksaFv5D1zYGCvDt2fEvDQWhVcMcaSieMmCSc54DDq3Rwh\nVersion: 1\nChain ID: 1\nNonce: Ki63qhXvxk0LYfxRE\nIssued At: 2021-12-08T13:09:59.716Z\nExpiration Time: 2021-12-08T13:24:59.715Z\nResources:\n- kepler://bafk2bzacedmmmpdngsjom66fob3gy3727fvc7dqqirlec3uyei7v2edmueazk#put\n- kepler://bafk2bzacedmmmpdngsjom66fob3gy3727fvc7dqqirlec3uyei7v2edmueazk#del\n- kepler://bafk2bzacedmmmpdngsjom66fob3gy3727fvc7dqqirlec3uyei7v2edmueazk#get\n- kepler://bafk2bzacedmmmpdngsjom66fob3gy3727fvc7dqqirlec3uyei7v2edmueazk#list","0x3c79ff9c565939bc4d43ac45d92f685de61b756a1ba9c0a8a5a80d177f05f29b7b27df1dc1c331397eef837d96b95dd812ce78c1b29a05c2b0c0bdd901be72351b"]"#;
    let message: SIWEMessage = serde_json::from_str(d)?;
    Ok(())
}
