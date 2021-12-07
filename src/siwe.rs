use crate::{
    auth::{Action, AuthorizationPolicy, AuthorizationToken},
    orbit::OrbitMetadata,
    zcap::KeplerInvocation,
};
use anyhow::Result;
use didkit::DID_METHODS;
use ipfs_embed::Cid;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use serde::{Deserialize, Serialize};
use serde_with::{hex::Hex, serde_as, DisplayFromStr};
use siwe::eip4361::Message;

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct SIWEDelegation(
    #[serde_as(as = "DisplayFromStr")] Message,
    #[serde_as(as = "Hex")] [u8; 65],
);

pub struct SIWETokens {
    pub invocation: KeplerInvocation,
    pub delegation: SIWEDelegation,
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
            (Some(Err(e)), _) => Outcome::Failure((Status::Unauthorized, e)),
            (_, Some(Err(e))) => Outcome::Failure((Status::Unauthorized, e)),
            (_, _) => Outcome::Forward(()),
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
        // check delegation to invoker
        if auth_token.delegation.0.uri.as_str()
            != auth_token
                .invocation
                .proof
                .as_ref()
                .and_then(|proof| proof.verification_method.as_ref())
                .ok_or_else(|| anyhow!("Missing delegation verification method"))?
        {
            return Err(anyhow!("Invoker not authorized"));
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
            .verify_eip191(auth_token.delegation.1)?;

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

#[test]
async fn basic() -> Result<()> {
    Ok(())
}
