use crate::{
    auth::{check_orbit_and_service, simple_prefix_check, AuthorizationPolicy, AuthorizationToken},
    manifest::Manifest,
    resource::ResourceId,
    zcap::KeplerInvocation,
};
use anyhow::Result;
use didkit::DID_METHODS;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};

use ethers_core::{types::H160, utils::to_checksum};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use siwe::Message;
use std::{ops::Deref, str::FromStr};

pub struct SIWESignature([u8; 65]);

impl core::fmt::Display for SIWESignature {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0))
    }
}

impl FromStr for SIWESignature {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
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
    pub invoked_action: ResourceId,
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
            (Some(Err(e)), _) => Outcome::Failure((Status::Unauthorized, e)),
            (_, Some(Err(e))) => Outcome::Failure((Status::Unauthorized, e)),
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
            (Some(Ok(i)), _) => (i, None),
            (Some(Err(e)), _) => return Outcome::Failure((Status::Unauthorized, e)),
            (None, Err(e)) => return Outcome::Failure((Status::Unauthorized, e)),
            (None, _) => return Outcome::Forward(()),
        };
        Outcome::Success(Self {
            invoked_action: match invocation
                .resources
                .iter()
                .find_map(|r| r.as_str().parse().ok())
            {
                Some(o) => o,
                None => {
                    return Outcome::Failure((Status::Unauthorized, anyhow!("Missing Resource")))
                }
            },
            invocation,
            delegation,
        })
    }
}

impl AuthorizationToken for SIWEZcapTokens {
    fn resource(&self) -> &ResourceId {
        &self.invocation.property_set.invocation_target
    }
}

impl AuthorizationToken for SIWETokens {
    fn resource(&self) -> &ResourceId {
        &self.invoked_action
    }
}

#[rocket::async_trait]
impl AuthorizationPolicy<SIWEZcapTokens> for Manifest {
    async fn authorize(&self, auth_token: &SIWEZcapTokens) -> Result<()> {
        // check delegator is controller
        if !self.delegators().contains(
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
        if format!("urn:siwe:kepler:{}", auth_token.delegation.nonce)
            != auth_token
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

        let target = &auth_token.invocation.property_set.invocation_target;

        if !auth_token
            .delegation
            .resources
            .iter()
            .filter_map(|s| s.as_str().parse().ok())
            .any(|r| {
                check_orbit_and_service(&target, &r).is_ok()
                    && simple_prefix_check(&target, &r).is_ok()
            })
        {
            return Err(anyhow!("Delegation semantics violated"));
        }

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
impl AuthorizationPolicy<SIWETokens> for Manifest {
    async fn authorize(&self, t: &SIWETokens) -> Result<()> {
        if t.invoked_action.orbit() != self.id() {
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
                if !self.delegators().contains(
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

                if d.uri != invoker
                    && !(d.uri.as_str().ends_with('*')
                        && invoker.starts_with(&d.uri.as_str()[..&d.uri.as_str().len() - 1]))
                {
                    return Err(anyhow!("Invoker not authorized"));
                };

                if !d
                    .resources
                    .iter()
                    .filter_map(|s| s.as_str().parse().ok())
                    .any(|r| {
                        check_orbit_and_service(&t.invoked_action, &r).is_ok()
                            && simple_prefix_check(&t.invoked_action, &r).is_ok()
                    })
                {
                    return Err(anyhow!("Delegation semantics violated"));
                };
            }
            None => {
                // check invoker is controller
                if !self.invokers().contains(&invoker.parse()?) {
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
async fn basic() {
    use crate::tracing_try_init;
    use rocket::{build, http::Header, local::asynchronous::Client};

    tracing_try_init();
    let d = base64::encode_config(
        r#"["test.org wants you to sign in with your Ethereum account:\n0x6c3Ca9380307EEDa246B7606B43b33F3e0786C79\n\nAuthorize this provider to host your Orbit\n\nURI: peer:12D3KooWSJT2PD5c1rEAD959q9kChGcWUnkkUzku28uY5pqegkuW\nVersion: 1\nChain ID: 1\nNonce: 3A9S4Ar7YibfspTb2\nIssued At: 2022-03-16T15:03:36.775Z\nExpiration Time: 2022-03-16T15:05:36.775Z\nResources:\n- kepler:pkh:eip155:1:0x6c3Ca9380307EEDa246B7606B43b33F3e0786C79://default#peer","0x6909694c1afe49fbe9350da8f89333397657ae46d9898dccdef45a38cf39e8fd1527e81e79b52db3f2ad2239c07cab8888f4882faf453c032c0e4df3d9c4902d1b"]"#,
        base64::URL_SAFE,
    );

    let client = Client::untracked(build().ignite().await.unwrap())
        .await
        .unwrap();
    let mut req = client.post("/");
    req.add_header(Header::new("x-siwe-invocation", d));
    let t = SIWETokens::from_request(req.inner()).await;
    assert!(t.is_success());
}
