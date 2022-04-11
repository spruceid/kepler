use crate::capabilities::store::Updates;
use crate::capabilities::{store::AuthRef, Invoke};
use crate::config;
use crate::manifest::Manifest;
use crate::orbit::{create_orbit, load_orbit, AuthTokens, Orbit};
use crate::relay::RelayNode;
use crate::resource::{OrbitId, ResourceId};
use crate::routes::Metadata;
use crate::siwe::SIWEDelegation;
use anyhow::Result;
use ipfs::{Multiaddr, PeerId};
use libp2p::identity::ed25519::Keypair as Ed25519Keypair;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use std::convert::TryInto;
use std::str::FromStr;
use std::{collections::HashMap, sync::RwLock};
use thiserror::Error;

pub trait AuthorizationToken {
    fn resource(&self) -> &ResourceId;
}

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

#[rocket::async_trait]
pub trait AuthorizationPolicy<T> {
    async fn authorize(&self, auth_token: &T) -> Result<()>;
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

pub struct DelegateAuthWrapper;

#[async_trait]
impl<'l> FromRequest<'l> for DelegateAuthWrapper {
    type Error = anyhow::Error;

    async fn from_request(req: &'l Request<'_>) -> Outcome<Self, Self::Error> {
        let (config, relay) = match get_state(req) {
            Ok(s) => s,
            Err(e) => return internal_server_error(e),
        };

        // TODO: Support only zcaps.
        let token = match SIWEDelegation::from_request(req).await {
            Outcome::Success(t) => t,
            Outcome::Failure(e) => return Outcome::Failure(e),
            Outcome::Forward(_) => return unauthorized(anyhow!("no delegation found")),
        };

        let resource = match token
            .delegation
            .resources
            .first()
            .ok_or_else(|| anyhow!("delegation has empty resource list"))
            .map(|uri| uri.as_str())
            .map(ResourceId::from_str)
        {
            Err(e) => return bad_request(e),
            Ok(Err(e)) => return bad_request(e),
            Ok(Ok(resource)) => resource,
        };
        let orbit_id = resource.orbit();

        //let token = match ZCAPDelegation::from_request(req).await {
        //    Outcome::Success(t) => t,
        //    Outcome::Failure(e) => return Outcome::Failure(e),
        //    Outcome::Forward(_) => return unauthorized(anyhow!("no delegation found")),
        //};

        //let orbit_id = match token.delegation.property_set.capability_action.first() {
        //    None => return unauthorized(anyhow!("delegation has empty capability action list")),
        //    Some(resource) => resource.orbit(),
        //};

        let orbit = match load_orbit(orbit_id.get_cid(), &config, relay).await {
            Ok(Some(o)) => o,
            Ok(None) => return Outcome::Failure((Status::NotFound, anyhow!("No Orbit found"))),
            Err(e) => return Outcome::Failure((Status::InternalServerError, e)),
        };

        let delegation = match (orbit_id.to_string().into(), token.delegation).try_into() {
            Err(e) => return unauthorized(e),
            Ok(d) => d,
        };

        //let delegation = match (token.delegation).try_into() {
        //    Err(e) => return unauthorized(e),
        //    Ok(d) => d,
        //};

        if let Err(e) = orbit
            .capabilities
            .transact(Updates::new([delegation], []))
            .await
        {
            return internal_server_error(e);
        }
        Outcome::Success(Self)
    }
}

pub enum InvokeAuthWrapper {
    Create(OrbitId),
    S3(Box<S3Action>),
}

pub enum S3Action {
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

#[async_trait]
impl<'l> FromRequest<'l> for InvokeAuthWrapper {
    type Error = anyhow::Error;

    async fn from_request(req: &'l Request<'_>) -> Outcome<Self, Self::Error> {
        let (config, relay) = match get_state(req) {
            Ok(s) => s,
            Err(e) => return internal_server_error(e),
        };

        let token = match AuthTokens::from_request(req).await {
            Outcome::Success(t) => t,
            Outcome::Failure(e) => return Outcome::Failure(e),
            Outcome::Forward(_) => return unauthorized(anyhow!("missing invocation token")),
        };

        let target = token.resource();

        match target.fragment() {
            None => unauthorized(anyhow!("target resource is missing action")),
            // TODO: Refactor '#peer` invocations to be delegations to the peer id.
            Some("peer") => {
                match (target.path(), target.service()) {
                    (None, None) => (),
                    _ => return bad_request(anyhow!("token action not matching endpoint")),
                }

                let keys = match req
                    .rocket()
                    .state::<RwLock<HashMap<PeerId, Ed25519Keypair>>>()
                {
                    Some(k) => k,
                    _ => return internal_server_error(anyhow!("could not retrieve open key set")),
                };

                let orbit_id = target.orbit().clone();

                let md = match Manifest::resolve_dyn(&orbit_id, None).await {
                    Ok(Some(md)) => md,
                    Ok(None) => return not_found(anyhow!("Orbit Manifest Doesnt Exist")),
                    Err(e) => return internal_server_error(e),
                };

                match md.authorize(&token).await {
                    Ok(()) => (),
                    Err(e) => return unauthorized(e),
                };

                // Do we even use this any more? It's just stored in the access_log on disk, which I think is unused?
                // Also the orbits I have on disk have empty access logs, so it seems we don't receive this header anyway (from the sdk).
                let auth_data = req
                    .headers()
                    .get_one("Authorization")
                    .unwrap_or("")
                    .as_bytes();

                let orbit = match create_orbit(&orbit_id, &config, auth_data, relay, keys).await {
                    Ok(Some(orbit)) => orbit,
                    Ok(None) => return conflict(anyhow!("Orbit already exists")),
                    Err(e) => return internal_server_error(e),
                };

                match orbit.invoke(&token).await {
                    Ok(_) => Outcome::Success(Self::Create(orbit_id)),
                    Err(e) => unauthorized(e),
                }
            }
            _ => {
                let orbit = match load_orbit(target.orbit().get_cid(), &config, relay).await {
                    Ok(Some(o)) => o,
                    Ok(None) => return not_found(anyhow!("No Orbit found")),
                    Err(e) => return internal_server_error(e),
                };
                let auth_ref = match orbit.invoke(&token).await {
                    Ok(auth_ref) => auth_ref,
                    Err(e) => return unauthorized(e),
                };
                match target.service() {
                    None => bad_request(anyhow!("missing service in invocation target")),
                    Some("s3") => {
                        let key = match target.path() {
                            Some(path) => path.strip_prefix('/').unwrap_or(path).to_string(),
                            None => {
                                return bad_request(anyhow!("missing path in invocation target"))
                            }
                        };
                        match target.fragment() {
                            None => bad_request(anyhow!("missing action in invocation target")),
                            Some("del") => Outcome::Success(Self::S3(Box::new(S3Action::Delete {
                                orbit,
                                key,
                                auth_ref,
                            }))),
                            Some("get") => {
                                Outcome::Success(Self::S3(Box::new(S3Action::Get { orbit, key })))
                            }
                            Some("list") => {
                                Outcome::Success(Self::S3(Box::new(S3Action::List { orbit })))
                            }
                            Some("metadata") => {
                                Outcome::Success(Self::S3(Box::new(S3Action::Metadata {
                                    orbit,
                                    key,
                                })))
                            }
                            Some("put") => match Metadata::from_request(req).await {
                                Outcome::Success(metadata) => {
                                    Outcome::Success(Self::S3(Box::new(S3Action::Put {
                                        orbit,
                                        key,
                                        metadata,
                                        auth_ref,
                                    })))
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
