use crate::authorization::{Delegation, Invocation, Verifiable};
use crate::capabilities::store::{InvokeError, ToBlock, Updates};
use crate::config;
use crate::orbit::{create_orbit, load_orbit, Orbit};
use crate::relay::RelayNode;
use crate::routes::Metadata;
use crate::BlockStores;
use anyhow::Result;
use kepler_lib::{
    authorization::Query,
    libipld::Cid,
    resource::{OrbitId, ResourceId},
};
use libp2p::{
    core::{Multiaddr, PeerId},
    identity::ed25519::Keypair as Ed25519Keypair,
};
use rocket::{
    futures::future::try_join_all,
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use std::{collections::HashMap, sync::RwLock};
use thiserror::Error;
use tracing::{info_span, Instrument};

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
    Delegation(Cid),
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

        let p = token
            .delegate
            .strip_prefix("peer:")
            .and_then(|s| s.parse().ok());
        // get relevant orbit IDs and whether the delegation is a host del
        let orbit_ids = token
            .capabilities
            .iter()
            .fold(HashMap::new(), |mut o, cap| {
                let r = &cap.resource;
                let peers = o.entry(r.orbit()).or_insert(None);
                if let (None, None, None, Some(peer), None, "host") = (
                    r.fragment(),
                    r.path(),
                    r.service(),
                    p,
                    &peers,
                    cap.action.as_str(),
                ) {
                    *peers = Some(peer);
                };
                o
            });

        let cid = match token.to_block() {
            Ok(b) => *b.cid(),
            Err(e) => return internal_server_error(e),
        };
        // load or create orbits
        let orbits = match try_join_all(
            orbit_ids
                .iter()
                .zip(std::iter::repeat((config, relay, token.clone())))
                .map(|((orbit_id, peer), (config, relay, token))| async move {
                    match (
                        peer,
                        load_orbit(
                            (*orbit_id).clone(),
                            &config.storage.blocks,
                            &config.storage.indexes,
                            relay.clone(),
                        )
                        .await,
                    ) {
                        (Some(p), Ok(None)) => {
                            let keys = match req
                                .rocket()
                                .state::<RwLock<HashMap<PeerId, Ed25519Keypair>>>()
                            {
                                Some(k) => k,
                                _ => {
                                    return Err(internal_server_error(anyhow!(
                                        "could not retrieve open key set"
                                    )))
                                }
                            };

                            if let Err(e) = token
                                .verify(
                                    &crate::authorization::EmptyCollection,
                                    None,
                                    &orbit_id.did(),
                                )
                                .await
                            {
                                // TODO should match on the error, an Invalid JWS should return a 400
                                return Err(unauthorized(e));
                            };

                            let kp = match keys.write() {
                                Ok(mut keys) => match keys.remove(p) {
                                    Some(k) => k,
                                    _ => return Err(not_found(anyhow!("Peer ID Not Present"))),
                                },
                                Err(_) => {
                                    return Err(internal_server_error(anyhow!(
                                        "could not retrieve open key set"
                                    )))
                                }
                            };

                            match create_orbit(
                                orbit_id,
                                &config.storage.blocks,
                                &config.storage.indexes,
                                relay.clone(),
                                kp,
                            )
                            .await
                            {
                                Ok(Some(orbit)) => Ok(orbit),
                                Ok(None) => Err(conflict(anyhow!("Orbit already exists"))),
                                Err(e) => Err(internal_server_error(e)),
                            }
                        }
                        (_, Ok(None)) => Err(not_found(anyhow!("No Orbit found"))),
                        (_, Ok(Some(o))) => Ok(o),
                        (_, Err(e)) => Err(unauthorized(e)),
                    }
                }),
        )
        .await
        {
            Ok(o) => o,
            Err(e) => return e,
        };

        if let Err(e) = try_join_all(
            orbits
                .into_iter()
                .zip(std::iter::repeat(token.clone()))
                .map(|(orbit, t)| async move {
                    orbit.capabilities.transact(Updates::new([t], [])).await
                }),
        )
        .await
        .map_err(internal_server_error)
        {
            return e;
        };
        Outcome::Success(Self::Delegation(cid))
    }
}

pub enum InvokeAuthWrapper<B> {
    KV(Box<KVAction<B>>),
    Revocation,
    CapabilityQuery(Box<CapAction<B>>),
}

impl<B> InvokeAuthWrapper<B> {
    pub fn prometheus_label(&self) -> &str {
        match self {
            InvokeAuthWrapper::Revocation => "revoke_delegation",
            InvokeAuthWrapper::KV(kv) => kv.prometheus_label(),
            InvokeAuthWrapper::CapabilityQuery(_) => "query capabilities",
        }
    }
}

pub enum KVAction<B> {
    Delete {
        orbit: Orbit<B>,
        key: String,
        auth_ref: Cid,
    },
    Get {
        orbit: Orbit<B>,
        key: String,
    },
    List {
        orbit: Orbit<B>,
        prefix: String,
    },
    Metadata {
        orbit: Orbit<B>,
        key: String,
    },
    Put {
        orbit: Orbit<B>,
        key: String,
        metadata: Metadata,
        auth_ref: Cid,
    },
}

impl<B> KVAction<B> {
    pub fn prometheus_label(&self) -> &str {
        match self {
            KVAction::Delete { .. } => "kv_delete",
            KVAction::Get { .. } => "kv_get",
            KVAction::List { .. } => "kv_list",
            KVAction::Metadata { .. } => "kv_metadata",
            KVAction::Put { .. } => "kv_put",
        }
    }
}

pub enum CapAction<B> {
    Query {
        orbit: Orbit<B>,
        query: Query,
        invoker: String,
    },
}

async fn invoke(
    token: Invocation,
    config: &config::Config,
    relay: (PeerId, Multiaddr),
) -> Outcome<(Orbit<BlockStores>, Cid), anyhow::Error> {
    let target = &token.capability;
    let orbit = match load_orbit(
        target.resource.orbit().clone(),
        &config.storage.blocks,
        &config.storage.indexes,
        relay,
    )
    .await
    {
        Ok(Some(o)) => o,
        Ok(None) => return not_found(anyhow!("No Orbit found")),
        Err(e) => return internal_server_error(e),
    };
    let auth_ref = match orbit.capabilities.invoke([token]).await {
        Ok(c) => c,
        Err(InvokeError::Unauthorized(e)) => return unauthorized(e),
        Err(InvokeError::Other(e)) => {
            warn!("Invoke error: {}", e);
            match e.downcast_ref::<aws_sdk_dynamodb::Error>() {
                Some(aws_sdk_dynamodb::Error::TransactionCanceledException(
                    aws_sdk_dynamodb::error::TransactionCanceledException {
                        cancellation_reasons: Some(reasons),
                        ..
                    },
                )) => {
                    for reason in reasons {
                        if reason.code == Some("ThrottlingError".to_string()) {
                            return Outcome::Failure((Status::ServiceUnavailable, e));
                        }
                    }
                    return internal_server_error(e);
                }
                _ => return internal_server_error(e),
            };
        }
    };
    Outcome::Success((orbit, auth_ref))
}

#[async_trait]
impl<'l> FromRequest<'l> for InvokeAuthWrapper<BlockStores> {
    type Error = anyhow::Error;

    async fn from_request(req: &'l Request<'_>) -> Outcome<Self, Self::Error> {
        let req_span = req
            .local_cache(|| Option::<crate::tracing::TracingSpan>::None)
            .as_ref()
            .unwrap();
        let span = info_span!(parent: &req_span.0, "invoke_auth_wrapper");
        // Instrumenting async block to handle yielding properly
        async move {
            let timer = crate::prometheus::AUTHORIZATION_HISTOGRAM
                .with_label_values(&["invoke"])
                .start_timer();

            let (config, relay) = match get_state(req) {
                Ok(s) => s,
                Err(e) => return internal_server_error(e),
            };

            let token = match Invocation::from_request(req).await {
                Outcome::Success(t) => t,
                Outcome::Failure((s, e)) => return Outcome::Failure((s, e.into())),
                Outcome::Forward(_) => return bad_request(anyhow!("missing invocation token")),
            };

            let target = &token.capability;
            let invoker = token.invoker.clone();

            let res = match target.resource.service() {
                None => bad_request(anyhow!("missing service in invocation target")),
                Some("capabilities") => match target.action.as_str() {
                    "read" => invoke(token, &config, relay)
                        .await
                        .map(|(orbit, _auth_ref)| {
                            Self::CapabilityQuery(Box::new(CapAction::Query {
                                orbit,
                                query: Query::All,
                                invoker,
                            }))
                        }),
                    a => bad_request(anyhow!("unsupported action in invocation target {}", a)),
                },
                Some("kv") => {
                    let key = match target.resource.path() {
                        Some(path) => path.strip_prefix('/').unwrap_or(path).to_string(),
                        None => return bad_request(anyhow!("missing path in invocation target")),
                    };
                    match target.action.as_str() {
                        "del" => invoke(token, &config, relay)
                            .await
                            .map(|(orbit, auth_ref)| {
                                Self::KV(Box::new(KVAction::Delete {
                                    orbit,
                                    key,
                                    auth_ref,
                                }))
                            }),
                        "get" => invoke(token, &config, relay)
                            .await
                            .map(|(orbit, _)| Self::KV(Box::new(KVAction::Get { orbit, key }))),
                        "list" => invoke(token, &config, relay).await.map(|(orbit, _)| {
                            Self::KV(Box::new(KVAction::List { orbit, prefix: key }))
                        }),
                        "metadata" => invoke(token, &config, relay).await.map(|(orbit, _)| {
                            Self::KV(Box::new(KVAction::Metadata { orbit, key }))
                        }),
                        "put" => {
                            match Metadata::from_request(req).await {
                                Outcome::Success(metadata) => invoke(token, &config, relay)
                                    .await
                                    .map(|(orbit, auth_ref)| {
                                        Self::KV(Box::new(KVAction::Put {
                                            orbit,
                                            key,
                                            metadata,
                                            auth_ref,
                                        }))
                                    }),
                                Outcome::Failure(e) => Outcome::Failure(e),
                                Outcome::Forward(_) => internal_server_error(anyhow!(
                                    "unable to parse metadata from request"
                                )),
                            }
                        }
                        a => bad_request(anyhow!("unsupported action in invocation target {}", a)),
                    }
                }
                Some(s) => bad_request(anyhow!("unsupported service in invocation target {}", s)),
            };

            timer.observe_duration();
            res
        }
        .instrument(span)
        .await
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
