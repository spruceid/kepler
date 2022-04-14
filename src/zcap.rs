use crate::{
    auth::{simple_check, AuthorizationPolicy, AuthorizationToken},
    capabilities::{
        store::{IndexReferences, Invocation as CapInvocation, Updates},
        AuthRef, Invoke,
    },
    manifest::Manifest,
    orbit::Orbit,
    resource::ResourceId,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use didkit::DID_METHODS;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ssi::{
    did::DIDURL,
    vc::URI,
    zcap::{Delegation, Invocation},
};
use std::{collections::HashMap as Map, convert::TryInto, str::FromStr};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DelProps {
    pub capability_action: Vec<ResourceId>,
    pub expiration: Option<DateTime<Utc>>,
    #[serde(flatten)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_fields: Option<Map<String, Value>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InvProps {
    pub invocation_target: ResourceId,
    #[serde(flatten)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_fields: Option<Map<String, Value>>,
}

pub type KeplerInvocation = Invocation<InvProps>;
pub type KeplerDelegation = Delegation<(), DelProps>;

#[derive(Clone)]
pub struct ZCAPInvocation {
    pub invocation: KeplerInvocation,
    pub delegation: Option<KeplerDelegation>,
}

pub struct ZCAPDelegation {
    pub delegation: KeplerDelegation,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ZCAPInvocation {
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

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ZCAPDelegation {
    type Error = anyhow::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match request
            .headers()
            .get_one("x-kepler-delegation")
            .map(|b64| {
                base64::decode_config(b64, base64::URL_SAFE)
                    .map_err(|e| anyhow!(e))
                    .and_then(|s| serde_json::from_slice(&s).map_err(|e| anyhow!(e)))
            })
            .transpose()
        {
            Ok(Some(delegation)) => Outcome::Success(Self { delegation }),
            Ok(None) => Outcome::Forward(()),
            Err(e) => Outcome::Failure((Status::Unauthorized, e)),
        }
    }
}

impl AuthorizationToken for ZCAPInvocation {
    fn resource(&self) -> &ResourceId {
        &self.invocation.property_set.invocation_target
    }
}

#[rocket::async_trait]
impl Invoke<ZCAPInvocation> for Orbit {
    async fn invoke(&self, invocation: &ZCAPInvocation) -> Result<AuthRef> {
        self.authorize(invocation).await?;

        let inv: CapInvocation = invocation.invocation.clone().try_into()?;

        match &invocation.delegation {
            Some(d) => {
                self.capabilities
                    .transact(Updates::new([d.clone().try_into()?], []))
                    .await?
            }
            None => (),
        };
        let id = inv.id().to_vec();

        Ok(AuthRef::new(self.capabilities.invoke([inv]).await?, id))
    }
}

#[rocket::async_trait]
impl AuthorizationPolicy<ZCAPInvocation> for Manifest {
    async fn authorize(&self, auth_token: &ZCAPInvocation) -> Result<()> {
        let invoker_vm = auth_token
            .invocation
            .proof
            .as_ref()
            .and_then(|proof| proof.verification_method.as_ref())
            .ok_or_else(|| anyhow!("Missing delegation verification method"))
            .and_then(|s| DIDURL::from_str(s).map_err(|e| e.into()))?;
        let res = match &auth_token.delegation {
            Some(d) => {
                let delegator_vm = d
                    .proof
                    .as_ref()
                    .and_then(|proof| proof.verification_method.as_ref())
                    .ok_or_else(|| anyhow!("Missing delegation verification method"))
                    .and_then(|s| DIDURL::from_str(s).map_err(|e| e.into()))?;
                if !self.delegators().contains(&delegator_vm) {
                    return Err(anyhow!("Delegator not authorized"));
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

                let target = &auth_token.invocation.property_set.invocation_target;

                if !d
                    .property_set
                    .capability_action
                    .iter()
                    .any(|r| simple_check(target, r).is_ok())
                {
                    return Err(anyhow!("Delegation semantics violated"));
                }

                let mut res = d
                    .verify(Default::default(), DID_METHODS.to_resolver())
                    .await;
                let mut res2 = auth_token
                    .invocation
                    .verify(Default::default(), DID_METHODS.to_resolver(), d)
                    .await;
                res.append(&mut res2);
                res
            }
            None => {
                if !self.invokers().contains(&invoker_vm) {
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
    let inv_str = r#"{"@context":["https://w3id.org/security/v2",{"capabilityAction":{"@id":"sec:capabilityAction","@type":"@json"}}],"id":"uuid:8097ab5c-ebd6-4924-b659-5f8009429e4d","invocationTarget":"kepler:pkh:eip155:1:0x3401fBE360502F420D5c27CB8AED88E86cc4a726://default/ipfs/#list","proof":{"type":"Ed25519Signature2018","proofPurpose":"capabilityInvocation","verificationMethod":"did:key:z6MkuMN5NfBrN6YbGjzsc5ekSQBVGut3Q6inc8aEtY2AoHZj#z6MkuMN5NfBrN6YbGjzsc5ekSQBVGut3Q6inc8aEtY2AoHZj","created":"2022-03-21T13:59:14.455Z","jws":"eyJhbGciOiJFZERTQSIsImNyaXQiOlsiYjY0Il0sImI2NCI6ZmFsc2V9..ybqGJAhCtAPE97cZTLLvX5f5IzJtZLaCmrYAGosckwt9MT5A-ZRQfcZsdwrDUGND5lSTAIAvxWjCOvtMA1RVCw","capability":"kepler:pkh:eip155:1:0x3401fBE360502F420D5c27CB8AED88E86cc4a726://default"}}"#;
    let inv: KeplerInvocation = serde_json::from_str(inv_str)?;
    let res = inv.verify_signature(None, DID_METHODS.to_resolver()).await;
    assert!(res.errors.is_empty());
    Ok(())
}
