use crate::auth::{cid_serde, Action, AuthorizationPolicy, AuthorizationToken};
use anyhow::Result;
use ipfs_embed::Cid;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
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
            request.headers().get_one("X-Kepler-Invocation").map(|b64| {
                base64::decode_config(b64, base64::URL_SAFE_NO_PAD)
                    .map_err(|e| anyhow!(e))
                    .and_then(|s| serde_json::from_slice(&s).map_err(|e| anyhow!(e)))
            }),
            request
                .headers()
                .get_one("X-Kepler-Delegation")
                .map(|b64| {
                    base64::decode_config(b64, base64::URL_SAFE_NO_PAD)
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
            (None, Ok(None)) => Outcome::Forward(()),
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

#[test]
async fn basic() -> Result<()> {
    let inv_str = r#"{"@context":["https://w3id.org/security/v2",{"capabilityAction":{"@id":"sec:capabilityAction","@type":"@json"}}],"id":"urn:uuid:helo","capabilityAction":{"get":["z3v8BBKAGbGkuFU8TQq3J7k9XDs9udtMCic4KMS6HBxHczS1Tyv"]},"invocationTarget":"z3v8BBKAxmb5DPsoCsaucZZ26FzPSbLWDAGtpHSiKjA4AJLQ3my","proof":{"@context":{"TezosMethod2021":"https://w3id.org/security#TezosMethod2021","TezosSignature2021":{"@context":{"@protected":true,"@version":1.1,"challenge":"https://w3id.org/security#challenge","created":{"@id":"http://purl.org/dc/terms/created","@type":"http://www.w3.org/2001/XMLSchema#dateTime"},"domain":"https://w3id.org/security#domain","expires":{"@id":"https://w3id.org/security#expiration","@type":"http://www.w3.org/2001/XMLSchema#dateTime"},"id":"@id","nonce":"https://w3id.org/security#nonce","proofPurpose":{"@context":{"@protected":true,"@version":1.1,"assertionMethod":{"@container":"@set","@id":"https://w3id.org/security#assertionMethod","@type":"@id"},"authentication":{"@container":"@set","@id":"https://w3id.org/security#authenticationMethod","@type":"@id"},"id":"@id","type":"@type"},"@id":"https://w3id.org/security#proofPurpose","@type":"@vocab"},"proofValue":"https://w3id.org/security#proofValue","publicKeyJwk":{"@id":"https://w3id.org/security#publicKeyJwk","@type":"@json"},"type":"@type","verificationMethod":{"@id":"https://w3id.org/security#verificationMethod","@type":"@id"}},"@id":"https://w3id.org/security#TezosSignature2021"}},"type":"TezosSignature2021","proofPurpose":"capabilityInvocation","proofValue":"edsigtg5tr3rNwKQ9MmwsSCy8wx5NtU5ZwR51JDEFWb1Lhnq1xQwBxCz5UN2SGKWcWbzHSsBcdaBH8FHQhQNEmGr3LPhg47HAHr","verificationMethod":"did:pkh:tz:tz1WWXeGFgtARRLPPzT2qcpeiQZ8oQb6rBZd#TezosMethod2021","created":"2021-08-16T12:00:52.721Z","capability":"kepler://z3v8BBKAxmb5DPsoCsaucZZ26FzPSbLWDAGtpHSiKjA4AJLQ3my","publicKeyJwk":{"alg":"EdBlake2b","crv":"Ed25519","kty":"OKP","x":"pJMxEXxmUWrqHFX2Q6F1AdzhW9L7ityjVTJl5iLdMGw"}}}"#;

    let inv: KeplerInvocation = serde_json::from_str(inv_str)?;
    let res = inv.verify_signature(None, &did_pkh::DIDPKH).await;
    assert!(res.errors.is_empty());
    Ok(())
}
