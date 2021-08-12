use crate::auth::{cid_serde, Action, AuthorizationPolicy, AuthorizationToken};
use anyhow::Result;
use ipfs_embed::Cid;
use rocket::request::{FromRequest, Outcome, Request};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ssi::{
    did::DIDURL,
    zcap::{Delegation, Invocation},
};
use std::{collections::HashMap as Map, str::FromStr};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KeplerProps {
    #[serde(with = "cid_serde")]
    pub invocation_target: Cid,
    pub capability_action: Action,
    #[serde(flatten)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_fields: Option<Map<String, Value>>,
}

pub type KeplerInvocation = Invocation<KeplerProps>;

pub type KeplerDelegation = Delegation<(), KeplerProps>;

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
                .get_one("x-kepler-invocation")
                .and_then(|b64| base64::decode_config(b64, base64::URL_SAFE_NO_PAD).ok())
                .map(|s| serde_json::from_slice(&s)),
            request
                .headers()
                .get_one("x-kepler-delegation")
                .and_then(|b64| base64::decode_config(b64, base64::URL_SAFE_NO_PAD).ok())
                .map(|s| serde_json::from_slice(&s))
                .transpose(),
        ) {
            (Some(Ok(invocation)), Ok(delegation)) => Outcome::Success(Self {
                invocation,
                delegation,
            }),
            _ => Outcome::Forward(()),
        }
    }
}

impl AuthorizationToken for ZCAPTokens {
    fn action(&self) -> Action {
        self.invocation.property_set.capability_action.clone()
    }
    fn target_orbit(&self) -> &Cid {
        &self.invocation.property_set.invocation_target
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
