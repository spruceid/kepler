use crate::auth::{Action, AuthorizationPolicy, AuthorizationToken};
use anyhow::Result;
use ipfs_embed::Cid;
use rocket::request::{FromRequest, Outcome, Request};
use ssi::{
    did::DIDURL,
    zcap::{Delegation, Invocation},
};

pub type KeplerInvocation = Invocation<Action>;

pub type KeplerDelegation = Delegation<Action, ()>;

pub type ZCAPAuthorization = Vec<DIDURL>;

#[derive(Clone)]
pub struct ZCAPTokens {
    pub invocation: KeplerInvocation,
    pub delegation: KeplerDelegation,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ZCAPTokens {
    type Error = anyhow::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match (
            request
                .headers()
                .get_one("Invocation")
                .and_then(|b64| base64::decode_config(b64, base64::URL_SAFE_NO_PAD).ok())
                .map(|s| serde_json::from_str(s)),
            request
                .headers()
                .get_one("Delegation")
                .and_then(|b64| base64::decode_config(b64, base64::URL_SAFE_NO_PAD).ok())
                .map(|s| serde_json::from_str(s)),
        ) {
            (Some(Ok(invocation)), Some(Ok(delegation))) => Outcome::Success(Self {
                invocation,
                delegation,
            }),
            _ => Outcome::Forward(()),
        }
    }
}

impl AuthorizationToken<'_> for ZCAPTokens {
    fn action(&self) -> Action {
        self.invocation
            .capability_action
            // safest default but should never happen
            .unwrap_or_else(|| Action::List(Cid::default()))
            .clone()
    }
}

#[rocket::async_trait]
impl AuthorizationPolicy<'_> for ZCAPAuthorization {
    type Token = ZCAPTokens;

    async fn authorize<'a>(&self, auth_token: &'a Self::Token) -> Result<()> {
        let delegator_vm = auth_token
            .delegation
            .proof
            .and_then(|proof| proof.verification_method)
            .and_then(|s| DIDURL::from_str(s))
            .ok_or_else(|| anyhow!("Missing delegation verification method"))??;
        if !self.iter().any(|vm| vm == delegator_vm) {
            return Err(anyhow!("Delegator not authorized"));
        };
        let res = auth_token
            .invocation
            .verify(Default::default(), &did_pkh::DIDPKH, &auth_token.delegation)
            .await;
        if let Some(e) = res.errors.first() {
            Err(anyhow!(e))
        } else {
            Ok(())
        }
    }
}
