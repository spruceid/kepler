use crate::{
    auth::{Action, AuthorizationPolicy, AuthorizationToken},
    orbit::{hash_same, OrbitMetadata},
    zcap::KeplerInvocation,
};
use anyhow::Result;
use didkit::DID_METHODS;
use ipfs_embed::Cid;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};

use ethers_core::{types::H160, utils::to_checksum};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use siwe::eip4361::Message;
use std::{ops::Deref, str::FromStr};

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

impl Deref for SIWEMessage {
    type Target = Message;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct SIWEZcapTokens {
    pub invocation: KeplerInvocation,
    pub delegation: SIWEMessage,
}

pub struct SIWETokens {
    pub invocation: SIWEMessage,
    pub delegation: Option<SIWEMessage>,
    // kinda weird
    pub orbit: Cid,
    pub action: Action,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for SIWEZcapTokens {
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
impl<'r> FromRequest<'r> for SIWETokens {
    type Error = anyhow::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let (invocation, delegation): (SIWEMessage, Option<SIWEMessage>) = match (
            request.headers().get_one("x-siwe-invocation").map(|b64| {
                base64::decode_config(b64, base64::URL_SAFE)
                    .map_err(|e| anyhow!(e))
                    .and_then(|s| serde_json::from_slice(&s).map_err(|e| anyhow!(e)))
            }),
            request
                .headers()
                .get_one("x-siwe-delegation")
                .map(|b64| {
                    base64::decode_config(b64, base64::URL_SAFE)
                        .map_err(|e| anyhow!(e))
                        .and_then(|s| serde_json::from_slice(&s).map_err(|e| anyhow!(e)))
                })
                .transpose(),
        ) {
            (Some(Ok(i)), Ok(d)) => (i, d),
            (Some(Err(e)), _) => {
                tracing::debug!("{}", e);
                return Outcome::Failure((Status::Unauthorized, e));
            }
            (_, Err(e)) => {
                tracing::debug!("{}", e);
                return Outcome::Failure((Status::Unauthorized, e));
            }
            (_, _) => return Outcome::Forward(()),
        };
        let (orbit, action) = match invocation
            .resources
            .first()
            .and_then(|u| u.as_str().strip_prefix("kepler://"))
            .and_then(|p| p.split_once("#"))
            .map(|(op, a)| match op.rsplit_once("/") {
                Some((o, p)) => Ok((
                    Cid::from_str(o)?,
                    Some(p.strip_prefix("s3/").unwrap_or(p)),
                    a,
                )),
                _ => Ok((Cid::from_str(op)?, None, a)),
            }) {
            Some(Ok((o, _, "host"))) => (
                o,
                Action::Create {
                    parameters: invocation
                        .uri
                        .as_str()
                        .strip_prefix("kepler://")
                        .unwrap_or("")
                        .into(),
                    content: vec![],
                },
            ),
            Some(Ok((o, _, "list"))) => (o, Action::List),
            Some(Ok((o, Some(p), a))) => (
                o,
                match a {
                    "get" => Action::Get(vec![p.into()]),
                    "put" => Action::Put(vec![p.into()]),
                    "del" => Action::Del(vec![p.into()]),
                    x => {
                        return Outcome::Failure((
                            Status::Unauthorized,
                            anyhow!("Invalid Action: {}", x),
                        ))
                    }
                },
            ),
            Some(Ok(_)) => {
                return Outcome::Failure((Status::Unauthorized, anyhow!("Missing Path")))
            }
            Some(Err(e)) => return Outcome::Failure((Status::Unauthorized, e)),
            _ => return Outcome::Failure((Status::Unauthorized, anyhow!("Missing Action"))),
        };
        Outcome::Success(Self {
            invocation,
            delegation,
            orbit,
            action,
        })
    }
}

impl AuthorizationToken for SIWEZcapTokens {
    fn action(&self) -> &Action {
        &self.invocation.property_set.capability_action
    }
    fn target_orbit(&self) -> &Cid {
        &self.invocation.property_set.invocation_target
    }
}

impl AuthorizationToken for SIWETokens {
    fn action(&self) -> &Action {
        &self.action
    }
    fn target_orbit(&self) -> &Cid {
        &self.orbit
    }
}

#[rocket::async_trait]
impl AuthorizationPolicy<SIWEZcapTokens> for OrbitMetadata {
    async fn authorize(&self, auth_token: &SIWEZcapTokens) -> Result<()> {
        // check delegator is controller
        if !self.controllers().contains(
            &format!(
                "did:pkh:eip155:{}:{}#blockchainAccountId",
                &auth_token.delegation.0.chain_id,
                &to_checksum(&H160(auth_token.delegation.0.address), None)
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
        if auth_token.delegation.uri.as_str() != invoker {
            return Err(anyhow!("Invoker not authorized"));
        };

        // check invoker invokes delegation
        if &format!("urn:siwe:kepler:{}", auth_token.delegation.nonce)
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
        if !auth_token.delegation.valid_now() {
            return Err(anyhow!("Delegation has Expired"));
        };

        // check action is authorized by blanket root ("." as relative path against "kepler://<orbit-id>/") auth
        if !auth_token.delegation.resources.contains(&match auth_token
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
            .verify_eip191(&auth_token.delegation.1 .0)?;

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
impl AuthorizationPolicy<SIWETokens> for OrbitMetadata {
    async fn authorize(&self, t: &SIWETokens) -> Result<()> {
        if &t.orbit != self.id() {
            return Err(anyhow!("Incorrect Orbit ID"));
        };

        let invoker = format!(
            "did:pkh:eip155:{}:{}#blockchainAccountId",
            &t.invocation.chain_id,
            &to_checksum(&H160(t.invocation.address), None)
        );

        match &t.delegation {
            Some(d) => {
                // check delegator is controller
                if !self.controllers().contains(
                    &format!(
                        "did:pkh:eip155:{}:{}#blockchainAccountId",
                        &d.chain_id,
                        &to_checksum(&H160(d.address), None)
                    )
                    .parse()?,
                ) {
                    return Err(anyhow!("Delegator not authorized"));
                };
                d.verify_eip191(&d.1 .0)?;

                if &d.uri != &invoker
                    && !(d.uri.as_str().ends_with("*")
                        && invoker.starts_with(&d.uri.as_str()[..&d.uri.as_str().len() - 1]))
                {
                    return Err(anyhow!("Invoker not authorized"));
                };
            }
            None => {
                // check invoker is controller
                if !self.controllers().contains(&invoker.parse()?) {
                    return Err(anyhow!("Invoker not authorized as Controller"));
                };
            }
        };
        // check time validity
        if !t.invocation.valid_now()
            || !t.delegation.as_ref().map(|s| s.valid_now()).unwrap_or(true)
        {
            return Err(anyhow!("Message has Expired"));
        };

        t.invocation.verify_eip191(&t.invocation.1 .0)?;

        Ok(())
    }
}

#[test]
async fn basic() -> Result<()> {
    let d = r#"["localhost wants you to sign in with your Ethereum account:\n0xA391f7adD776806c4dFf3886BBe6370be8F73683\n\nAllow localhost to access your orbit using their temporary session key: did:key:z6MksaFv5D1zYGCvDt2fEvDQWhVcMcaSieMmCSc54DDq3Rwh#z6MksaFv5D1zYGCvDt2fEvDQWhVcMcaSieMmCSc54DDq3Rwh\n\nURI: did:key:z6MksaFv5D1zYGCvDt2fEvDQWhVcMcaSieMmCSc54DDq3Rwh#z6MksaFv5D1zYGCvDt2fEvDQWhVcMcaSieMmCSc54DDq3Rwh\nVersion: 1\nChain ID: 1\nNonce: Ki63qhXvxk0LYfxRE\nIssued At: 2021-12-08T13:09:59.716Z\nExpiration Time: 2021-12-08T13:24:59.715Z\nResources:\n- kepler://bafk2bzacedmmmpdngsjom66fob3gy3727fvc7dqqirlec3uyei7v2edmueazk#put\n- kepler://bafk2bzacedmmmpdngsjom66fob3gy3727fvc7dqqirlec3uyei7v2edmueazk#del\n- kepler://bafk2bzacedmmmpdngsjom66fob3gy3727fvc7dqqirlec3uyei7v2edmueazk#get\n- kepler://bafk2bzacedmmmpdngsjom66fob3gy3727fvc7dqqirlec3uyei7v2edmueazk#list","0x3c79ff9c565939bc4d43ac45d92f685de61b756a1ba9c0a8a5a80d177f05f29b7b27df1dc1c331397eef837d96b95dd812ce78c1b29a05c2b0c0bdd901be72351b"]"#;
    let message: SIWEMessage = serde_json::from_str(d)?;
    Ok(())
}
