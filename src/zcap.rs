use crate::auth::{Action, AuthorizationPolicy, AuthorizationToken};
use anyhow::Result;
use ipfs_embed::Cid;
use rocket::request::{FromRequest, Outcome, Request};
use ssi::{
    did::DIDURL,
    zcap::{DefaultProps, Delegation, Invocation},
};
use std::str::FromStr;

pub type KeplerInvocation = Invocation<DefaultProps<Action>>;

pub type KeplerDelegation = Delegation<(), DefaultProps<Action>>;

pub type ZCAPAuthorization = Vec<DIDURL>;

#[derive(Clone)]
pub struct ZCAPTokens {
    pub invocation: KeplerInvocation,
    pub delegation: Option<KeplerDelegation>,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ZCAPTokens {
    type Error = anyhow::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match (
            request
                .headers()
                .get_one("X-Kepler-Invocation")
                .and_then(|b64| base64::decode_config(b64, base64::URL_SAFE_NO_PAD).ok())
                .map(|s| serde_json::from_slice(&s)),
            request
                .headers()
                .get_one("X-Kepler-Delegation")
                .and_then(|b64| base64::decode_config(b64, base64::URL_SAFE_NO_PAD).ok())
                .map(|s| serde_json::from_slice(&s)),
        ) {
            (Some(Ok(invocation)), Some(Ok(delegation))) => Outcome::Success(Self {
                invocation,
                delegation,
            }),
            _ => Outcome::Forward(()),
        }
    }
}

impl AuthorizationToken for ZCAPTokens {
    fn action(&self) -> Action {
        self.invocation
            .property_set
            .capability_action
            .clone()
            // safest default but should never happen
            .unwrap_or_else(|| Action::List {
                orbit_id: Cid::default(),
            })
    }
}

#[rocket::async_trait]
impl AuthorizationPolicy for ZCAPAuthorization {
    type Token = ZCAPTokens;

    async fn authorize<'a>(&self, auth_token: &'a Self::Token) -> Result<()> {
        let res = match &auth_token.delegation {
            Some(d) => {
                let delegator_vm = d
                    .proof
                    .as_ref()
                    .and_then(|proof| proof.verification_method.as_ref())
                    .ok_or_else(|| anyhow!("Missing delegation verification method"))
                    .and_then(|s| DIDURL::from_str(&s).map_err(|e| e.into()))?;
                if !self.iter().any(|vm| vm == &delegator_vm) {
                    return Err(anyhow!("Delegator not authorized"));
                };
                auth_token
                    .invocation
                    .verify(Default::default(), &did_pkh::DIDPKH, &d)
                    .await
            }
            None => {
                let invoker_vm = auth_token
                    .invocation
                    .proof
                    .as_ref()
                    .and_then(|proof| proof.verification_method.as_ref())
                    .ok_or_else(|| anyhow!("Missing delegation verification method"))
                    .and_then(|s| DIDURL::from_str(&s).map_err(|e| e.into()))?;
                if !self.iter().any(|vm| vm == &invoker_vm) {
                    return Err(anyhow!("Delegator not authorized"));
                };
                auth_token
                    .invocation
                    .verify_signature(Default::default(), &did_pkh::DIDPKH)
                    .await
            }
        };

        res.errors
            .first()
            .map(|e| Err(anyhow!(e.clone())))
            .unwrap_or(Ok(()))
    }
}
