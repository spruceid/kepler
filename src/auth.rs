use crate::capabilities::store::{AuthRef, Updates};
use crate::config;
use crate::orbit::{create_orbit, load_orbit, Orbit};
use crate::relay::RelayNode;
use crate::resource::{OrbitId, ResourceId};
use crate::routes::Metadata;
use crate::zcap::{CapNode, Delegation, Invocation, Revocation, Verifiable};
use anyhow::Result;
use ipfs::{Multiaddr, PeerId};
use libp2p::identity::ed25519::Keypair as Ed25519Keypair;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use std::{collections::HashMap, sync::RwLock};
use thiserror::Error;

pub fn simple_check(target: &ResourceId, capability: &ResourceId) -> Result<()> {
    check_orbit_and_service(target, capability)?;
    simple_prefix_check(target, capability)?;
    simple_check_fragments(target, capability)
}

pub fn simple_check_fragments(target: &ResourceId, capability: &ResourceId) -> Result<()> {
    match (target.fragment(), capability.fragment()) {
        (Some(t), Some(c)) if t == c => Ok(()),
        _ => Err(anyhow!("Target Action does not match Capability")),
    }
}

pub fn simple_prefix_check(target: &ResourceId, capability: &ResourceId) -> Result<()> {
    // if #action is same
    // Ok if target.path => cap.path
    if target.service() == capability.service()
        && match (target.path(), capability.path()) {
            (Some(t), Some(c)) => t.starts_with(c),
            (_, None) => true,
            _ => false,
        }
    {
        Ok(())
    } else {
        Err(anyhow!("Target Service and Path are not correct"))
    }
}

#[derive(Error, Debug)]
pub enum TargetCheckError {
    #[error("Invocation and Capability Orbits do not match")]
    IncorrectOrbit,
    #[error("Invocation and Capability Services do not match")]
    IncorrectService,
}

pub fn check_orbit_and_service(
    target: &ResourceId,
    capability: &ResourceId,
) -> Result<(), TargetCheckError> {
    if target.orbit() != capability.orbit() {
        Err(TargetCheckError::IncorrectOrbit)
    } else if target.service() != capability.service() {
        Err(TargetCheckError::IncorrectService)
    } else {
        Ok(())
    }
}

fn get_state(req: &Request<'_>) -> Result<(config::Config, (PeerId, Multiaddr))> {
    Ok((
        req.rocket()
            .state::<config::Config>()
            .cloned()
            .ok_or_else(|| anyhow!("Could not retrieve config"))?,
        req.rocket()
            .state::<RelayNode>()
            .map(|r| (r.id, r.internal()))
            .ok_or_else(|| anyhow!("Could not retrieve relay node information"))?,
    ))
}

pub enum DelegateAuthWrapper {
    OrbitCreation(OrbitId),
    Delegation,
}

#[async_trait]
impl<'l> FromRequest<'l> for DelegateAuthWrapper {
    type Error = anyhow::Error;

    async fn from_request(req: &'l Request<'_>) -> Outcome<Self, Self::Error> {
        let (config, relay) = match get_state(req) {
            Ok(s) => s,
            Err(e) => return internal_server_error(e),
        };

        let token = match Delegation::from_request(req).await {
            Outcome::Success(t) => t,
            Outcome::Failure((s, e)) => return Outcome::Failure((s, e.into())),
            Outcome::Forward(_) => return unauthorized(anyhow!("no delegation found")),
        };

        let orbit = match (
            token
                .resources()
                .into_iter()
                .any(|r| (r.fragment(), r.path(), r.service()) == (Some("host"), None, None)),
            load_orbit(token.resource().orbit().get_cid(), &config, relay.clone()).await,
        ) {
            (true, Ok(None)) => {
                let keys = match req
                    .rocket()
                    .state::<RwLock<HashMap<PeerId, Ed25519Keypair>>>()
                {
                    Some(k) => k,
                    _ => return internal_server_error(anyhow!("could not retrieve open key set")),
                };

                if let Err(e) = token.verify(None).await {
                    return unauthorized(e);
                };

                let peer_id = match token.delegate().strip_prefix("peer:").map(|s| s.parse()) {
                    Some(Ok(p)) => p,
                    _ => return bad_request(anyhow!("Invalid Peer ID")),
                };

                let kp = match keys.write() {
                    Ok(mut keys) => match keys.remove(&peer_id) {
                        Some(k) => k,
                        _ => return not_found(anyhow!("Peer ID Not Present")),
                    },
                    Err(_) => {
                        return internal_server_error(anyhow!("could not retrieve open key set"))
                    }
                };

                match create_orbit(token.resource().orbit(), &config, &[], relay, kp).await {
                    Ok(Some(orbit)) => orbit,
                    Ok(None) => return conflict(anyhow!("Orbit already exists")),
                    Err(e) => return internal_server_error(e),
                }
            }
            (_, Ok(None)) => return not_found(anyhow!("No Orbit found")),
            (_, Ok(Some(o))) => o,
            (_, Err(e)) => return unauthorized(e),
        };

        if let Err(e) = orbit.capabilities.transact(Updates::new([token], [])).await {
            return internal_server_error(e);
        }
        Outcome::Success(Self::Delegation)
    }
}

pub enum KVAction {
    Delete {
        orbit: Orbit,
        key: String,
        auth_ref: AuthRef,
    },
    Get {
        orbit: Orbit,
        key: String,
    },
    List {
        orbit: Orbit,
    },
    Metadata {
        orbit: Orbit,
        key: String,
    },
    Put {
        orbit: Orbit,
        key: String,
        metadata: Metadata,
        auth_ref: AuthRef,
    },
}

pub enum InvokeAuthWrapper {
    KV(KVAction),
    Revocation,
}

#[async_trait]
impl<'l> FromRequest<'l> for InvokeAuthWrapper {
    type Error = anyhow::Error;

    async fn from_request(req: &'l Request<'_>) -> Outcome<Self, Self::Error> {
        let (config, relay) = match get_state(req) {
            Ok(s) => s,
            Err(e) => return internal_server_error(e),
        };

        let token = match Invocation::from_request(req).await {
            Outcome::Success(t) => t,
            Outcome::Failure((s, e)) => return Outcome::Failure((s, e.into())),
            Outcome::Forward(_) => return unauthorized(anyhow!("missing invocation token")),
        };

        let target = token.resource();

        match target.fragment() {
            None => unauthorized(anyhow!("target resource is missing action")),
            _ => {
                let orbit = match load_orbit(target.orbit().get_cid(), &config, relay).await {
                    Ok(Some(o)) => o,
                    Ok(None) => return not_found(anyhow!("No Orbit found")),
                    Err(e) => return internal_server_error(e),
                };
                match target.service() {
                    None => bad_request(anyhow!("missing service in invocation target")),
                    Some("kv") => {
                        let tid = token.id().to_vec();
                        let auth_ref = match orbit.capabilities.invoke([token.clone()]).await {
                            Ok(c) => AuthRef::new(c, tid),
                            Err(e) => return unauthorized(e),
                        };

                        let key = match target.path() {
                            Some(path) => path.strip_prefix('/').unwrap_or(path).to_string(),
                            None => {
                                return bad_request(anyhow!("missing path in invocation target"))
                            }
                        };
                        match target.fragment() {
                            None => bad_request(anyhow!("missing action in invocation target")),
                            Some("del") => Outcome::Success(Self::KV(KVAction::Delete {
                                orbit,
                                key,
                                auth_ref,
                            })),
                            Some("get") => Outcome::Success(Self::KV(KVAction::Get { orbit, key })),
                            Some("list") => Outcome::Success(Self::KV(KVAction::List { orbit })),
                            Some("metadata") => {
                                Outcome::Success(Self::KV(KVAction::Metadata { orbit, key }))
                            }
                            Some("put") => match Metadata::from_request(req).await {
                                Outcome::Success(metadata) => {
                                    Outcome::Success(Self::KV(KVAction::Put {
                                        orbit,
                                        key,
                                        metadata,
                                        auth_ref,
                                    }))
                                }
                                Outcome::Failure(e) => Outcome::Failure(e),
                                Outcome::Forward(_) => internal_server_error(anyhow!(
                                    "unable to parse metadata from request"
                                )),
                            },
                            Some(a) => bad_request(anyhow!(
                                "unsupported action in invocation target {}",
                                a
                            )),
                        }
                    }
                    Some(s) => {
                        bad_request(anyhow!("unsupported service in invocation target {}", s))
                    }
                }
            }
        }
    }
}

fn bad_request<T, E: Into<anyhow::Error>>(e: E) -> Outcome<T, anyhow::Error> {
    Outcome::Failure((Status::BadRequest, e.into()))
}

fn conflict<T, E: Into<anyhow::Error>>(e: E) -> Outcome<T, anyhow::Error> {
    Outcome::Failure((Status::Conflict, e.into()))
}

fn internal_server_error<T, E: Into<anyhow::Error>>(e: E) -> Outcome<T, anyhow::Error> {
    Outcome::Failure((Status::InternalServerError, e.into()))
}

fn not_found<T, E: Into<anyhow::Error>>(e: E) -> Outcome<T, anyhow::Error> {
    Outcome::Failure((Status::NotFound, e.into()))
}

fn unauthorized<T, E: Into<anyhow::Error>>(e: E) -> Outcome<T, anyhow::Error> {
    Outcome::Failure((Status::Unauthorized, e.into()))
}
