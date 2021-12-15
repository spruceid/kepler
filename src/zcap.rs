use crate::{
    auth::{Action, AuthorizationPolicy, AuthorizationToken},
    orbit::OrbitMetadata,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use didkit::DID_METHODS;
use ipfs_embed::Cid;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::{serde_as, DisplayFromStr};
use ssi::{
    did::DIDURL,
    vc::URI,
    zcap::{Delegation, Invocation},
};
use std::{collections::HashMap as Map, str::FromStr};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DelProps {
    pub capability_action: Vec<String>,
    pub expiration: Option<DateTime<Utc>>,
    #[serde(flatten)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_fields: Option<Map<String, Value>>,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InvProps {
    #[serde_as(as = "DisplayFromStr")]
    pub invocation_target: Cid,
    pub capability_action: Action,
    #[serde(flatten)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_fields: Option<Map<String, Value>>,
}

pub type KeplerInvocation = Invocation<InvProps>;
pub type KeplerDelegation = Delegation<(), DelProps>;

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
            request.headers().get_one("x-kepler-invocation").map(|b64| {
                base64::decode_config(b64, base64::URL_SAFE)
                    .map_err(|e| anyhow!(e))
                    .and_then(|s| serde_json::from_slice(&s).map_err(|e| anyhow!(e)))
            }),
            request
                .headers()
                .get_one("x-kepler-delegation")
                .map(|b64| {
                    base64::decode_config(b64, base64::URL_SAFE)
                        .map_err(|e| anyhow!(e))
                        .and_then(|s| serde_json::from_slice(&s).map_err(|e| anyhow!(e)))
                })
                .transpose(),
        ) {
            (Some(Ok(invocation)), Ok(delegation)) => Outcome::Success(Self {
                invocation,
                delegation,
            }),
            (Some(Err(e)), _) => Outcome::Failure((Status::Unauthorized, e)),
            (_, Err(e)) => Outcome::Failure((Status::Unauthorized, e)),
            (None, _) => Outcome::Forward(()),
        }
    }
}

impl AuthorizationToken for ZCAPTokens {
    fn action(&self) -> &Action {
        &self.invocation.property_set.capability_action
    }
    fn target_orbit(&self) -> &Cid {
        &self.invocation.property_set.invocation_target
    }
}

#[rocket::async_trait]
impl AuthorizationPolicy<ZCAPTokens> for OrbitMetadata {
    async fn authorize(&self, auth_token: &ZCAPTokens) -> Result<()> {
        let invoker_vm = auth_token
            .invocation
            .proof
            .as_ref()
            .and_then(|proof| proof.verification_method.as_ref())
            .ok_or_else(|| anyhow!("Missing delegation verification method"))
            .and_then(|s| DIDURL::from_str(&s).map_err(|e| e.into()))?;
        let res = match &auth_token.delegation {
            Some(d) => {
                let delegator_vm = d
                    .proof
                    .as_ref()
                    .and_then(|proof| proof.verification_method.as_ref())
                    .ok_or_else(|| anyhow!("Missing delegation verification method"))
                    .and_then(|s| DIDURL::from_str(&s).map_err(|e| e.into()))?;
                match auth_token.invocation.property_set.capability_action {
                    Action::List | Action::Get(_) => {
                        if !self.read_delegators.contains(&delegator_vm)
                            && !self.write_delegators.contains(&delegator_vm)
                            && !self.controllers.contains(&delegator_vm)
                        {
                            return Err(anyhow!("Delegator not authorized"));
                        }
                    }
                    Action::Put(_) | Action::Del(_) => {
                        if !self.write_delegators.contains(&delegator_vm)
                            && !self.controllers.contains(&delegator_vm)
                        {
                            return Err(anyhow!("Delegator not write-authorized"));
                        }
                    }
                    _ => return Err(anyhow!("Invalid Action")),
                };
                if let Some(ref authorized_invoker) = d.invoker {
                    if authorized_invoker != &URI::String(invoker_vm.to_string()) {
                        return Err(anyhow!("Invoker not authorized"));
                    };
                };
                if let Some(exp) = d.property_set.expiration {
                    if exp < Utc::now() {
                        return Err(anyhow!("Delegation has Expired"));
                    }
                };
                if !d.property_set.capability_action.contains(&match auth_token
                    .invocation
                    .property_set
                    .capability_action
                {
                    Action::List => "list".into(),
                    Action::Put(_) => "put".into(),
                    Action::Get(_) => "get".into(),
                    Action::Del(_) => "del".into(),
                    _ => return Err(anyhow!("Invalid Action")),
                }) {
                    return Err(anyhow!("Invoked action not authorized by delegation"));
                };
                let mut res = d
                    .verify(Default::default(), DID_METHODS.to_resolver())
                    .await;
                let mut res2 = auth_token
                    .invocation
                    .verify(Default::default(), DID_METHODS.to_resolver(), &d)
                    .await;
                res.append(&mut res2);
                res
            }
            None => {
                if !self.controllers.contains(&invoker_vm) {
                    return Err(anyhow!("Invoker not authorized as Controller"));
                };
                auth_token
                    .invocation
                    .verify_signature(Default::default(), DID_METHODS.to_resolver())
                    .await
            }
        };

        res.errors
            .first()
            .map(|e| Err(anyhow!(e.clone())))
            .unwrap_or(Ok(()))
    }
}

#[test]
async fn basic() -> Result<()> {
    let del_str = r#"{"@context":["https://w3id.org/security/v2",{"capabilityAction":{"@id":"sec:capabilityAction","@type":"@json"}}],"id":"uuid:bac4da68-eb75-446b-8f9f-87608cbf872b","parentCapability":"kepler://zCT5htkeDnBhDwQ9JsPnZKuzzQG6fSe3U44oCjZ5tkAPyNvPVXvg","invoker":"did:key:z6MkmhGnWtb1bo18Z3QfvKXFxRp6e3LHmG7i8z7ZkAa39tKA#z6MkmhGnWtb1bo18Z3QfvKXFxRp6e3LHmG7i8z7ZkAa39tKA","capabilityAction":["get","list","put","del"],"expiration":"2021-09-08T17:01:13.991Z","proof":{"@context":{"TezosMethod2021":"https://w3id.org/security#TezosMethod2021","TezosSignature2021":{"@context":{"@protected":true,"@version":1.1,"challenge":"https://w3id.org/security#challenge","created":{"@id":"http://purl.org/dc/terms/created","@type":"http://www.w3.org/2001/XMLSchema#dateTime"},"domain":"https://w3id.org/security#domain","expires":{"@id":"https://w3id.org/security#expiration","@type":"http://www.w3.org/2001/XMLSchema#dateTime"},"id":"@id","nonce":"https://w3id.org/security#nonce","proofPurpose":{"@context":{"@protected":true,"@version":1.1,"assertionMethod":{"@container":"@set","@id":"https://w3id.org/security#assertionMethod","@type":"@id"},"authentication":{"@container":"@set","@id":"https://w3id.org/security#authenticationMethod","@type":"@id"},"id":"@id","type":"@type"},"@id":"https://w3id.org/security#proofPurpose","@type":"@vocab"},"proofValue":"https://w3id.org/security#proofValue","publicKeyJwk":{"@id":"https://w3id.org/security#publicKeyJwk","@type":"@json"},"type":"@type","verificationMethod":{"@id":"https://w3id.org/security#verificationMethod","@type":"@id"}},"@id":"https://w3id.org/security#TezosSignature2021"}},"type":"TezosSignature2021","proofPurpose":"capabilityDelegation","proofValue":"edsigtXsZpmWpUqm5eNgFehmnpRbFVuJsLTTwDvrYkK8pmswpTxKFCUhDyfjjs13Gw6oGtBkgJxSMECdfvpN49pCruyokvQrg41","verificationMethod":"did:pkh:tz:tz1auyCb6BDGYqZL38UqpazAoHrztt197Tfr#TezosMethod2021","created":"2021-09-08T17:00:13.995Z","publicKeyJwk":{"alg":"EdBlake2b","crv":"Ed25519","kty":"OKP","x":"aEofZ76eliz8VX4ys9XsR1q3HXQ4sGsPT9p00kx-SLU"},"capabilityChain":[]}}"#;
    let del: KeplerDelegation = serde_json::from_str(del_str)?;
    let res = del.verify(None, DID_METHODS.to_resolver()).await;
    assert!(res.errors.is_empty());
    Ok(())
}
