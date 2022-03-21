use crate::cas::CidWrap;
use crate::config;
use crate::manifest::Manifest;
use crate::orbit::{create_orbit, hash_same, load_orbit, AuthTokens, Orbit};
use crate::relay::RelayNode;
use crate::resource::ResourceId;
use anyhow::Result;
use ipfs::{Multiaddr, PeerId};
use libipld::cid::Cid;
use libp2p::identity::ed25519::Keypair as Ed25519Keypair;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use std::{collections::HashMap, sync::RwLock};
use thiserror::Error;

pub trait AuthorizationToken {
    fn resource(&self) -> &ResourceId;
}

pub fn simple_prefix_check(target: &ResourceId, capability: &ResourceId) -> Result<()> {
    // if #action is same
    // Ok if target.path => cap.path
    if target.service() == capability.service()
        && match (target.path(), capability.path()) {
            (Some(t), Some(c)) => t.starts_with(c),
            (Some(_), None) => true,
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
    tracing::debug!("{} {}", target, capability);
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

pub struct PutAuthWrapper(pub Orbit);
pub struct GetAuthWrapper(pub Orbit);
pub struct MetadataAuthWrapper(pub Orbit);
pub struct DelAuthWrapper(pub Orbit);
pub struct CreateAuthWrapper(pub Orbit);
pub struct ListAuthWrapper(pub Orbit);

async fn extract_info<T>(
    req: &Request<'_>,
) -> Result<(Vec<u8>, AuthTokens, config::Config, (PeerId, Multiaddr)), Outcome<T, anyhow::Error>> {
    // TODO need to identify auth method from the headers
    let auth_data = req.headers().get_one("Authorization").unwrap_or("");
    let config = match req.rocket().state::<config::Config>() {
        Some(c) => c,
        None => {
            return Err(Outcome::Failure((
                Status::InternalServerError,
                anyhow!("Could not retrieve config"),
            )));
        }
    };
    let relay = match req.rocket().state::<RelayNode>() {
        Some(r) => (r.id, r.internal()),
        _ => {
            return Err(Outcome::Failure((
                Status::InternalServerError,
                anyhow!("Could not retrieve Relay Node information"),
            )));
        }
    };
    match AuthTokens::from_request(req).await {
        Outcome::Success(token) => {
            Ok((auth_data.as_bytes().to_vec(), token, config.clone(), relay))
        }
        Outcome::Failure(e) => Err(Outcome::Failure(e)),
        Outcome::Forward(_) => Err(Outcome::Failure((
            Status::Unauthorized,
            anyhow!("No valid authorization headers"),
        ))),
    }
}

// TODO some APIs prefer to return 404 when the authentication fails to avoid leaking information about content

macro_rules! impl_fromreq {
    ($type:ident, $method:tt) => {
        #[rocket::async_trait]
        impl<'r> FromRequest<'r> for $type {
            type Error = anyhow::Error;

            async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
                let (_, token, config, relay) = match extract_info(req).await {
                    Ok(i) => i,
                    Err(o) => return o,
                };
                let oid: Cid = match req.param::<CidWrap>(0) {
                    Some(Ok(o)) => o.0,
                    _ => {
                        return Outcome::Failure((
                            Status::BadRequest,
                            anyhow!("Could not parse orbit"),
                        ));
                    }
                };
                match (
                    token.resource().fragment().as_ref().map(|s| s.as_str()),
                    &oid == &match hash_same(&oid, token.resource().orbit().to_string()) {
                        Ok(c) => c,
                        Err(_) => {
                            return Outcome::Failure((
                                Status::BadRequest,
                                anyhow!("Could not match orbit"),
                            ))
                        }
                    },
                ) {
                    (_, false) => Outcome::Failure((
                        Status::BadRequest,
                        anyhow!("Token target orbit not matching endpoint"),
                    )),
                    (Some($method), true) => {
                        let orbit = match load_orbit(
                            token.resource().orbit().get_cid(),
                            config.database.path.clone(),
                            relay,
                        )
                        .await
                        {
                            Ok(Some(o)) => o,
                            Ok(None) => {
                                return Outcome::Failure((
                                    Status::NotFound,
                                    anyhow!("No Orbit found"),
                                ))
                            }
                            Err(e) => return Outcome::Failure((Status::InternalServerError, e)),
                        };
                        match orbit.authorize(&token).await {
                            Ok(_) => Outcome::Success(Self(orbit)),
                            Err(e) => Outcome::Failure((Status::Unauthorized, e)),
                        }
                    }
                    _ => Outcome::Failure((
                        Status::BadRequest,
                        anyhow!("Token action not matching endpoint"),
                    )),
                }
            }
        }
    };
}

impl_fromreq!(PutAuthWrapper, "put");
impl_fromreq!(GetAuthWrapper, "get");
impl_fromreq!(MetadataAuthWrapper, "metadata");
impl_fromreq!(DelAuthWrapper, "del");
impl_fromreq!(ListAuthWrapper, "list");

#[rocket::async_trait]
impl<'r> FromRequest<'r> for CreateAuthWrapper {
    type Error = anyhow::Error;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let (auth_data, token, config, relay) = match extract_info(req).await {
            Ok(i) => i,
            Err(o) => return o,
        };
        let keys = match req
            .rocket()
            .state::<RwLock<HashMap<PeerId, Ed25519Keypair>>>()
        {
            Some(k) => k,
            _ => {
                return Outcome::Failure((
                    Status::InternalServerError,
                    anyhow!("Could not retrieve open key set"),
                ));
            }
        };

        match (
            token.resource().fragment().as_ref().map(|s| s.as_str()),
            token.resource().path(),
            token.resource().service(),
        ) {
            // Create actions dont have an existing orbit to authorize against, it's a node policy
            // TODO have policy config, for now just be very permissive :shrug:
            (Some("peer"), None, None) => {
                let md = match Manifest::resolve_dyn(token.resource().orbit(), None).await {
                    Ok(Some(md)) => md,
                    Ok(None) => {
                        return Outcome::Failure((
                            Status::NotFound,
                            anyhow!("Orbit Manifest Doesnt Exist"),
                        ))
                    }
                    Err(e) => return Outcome::Failure((Status::InternalServerError, anyhow!(e))),
                };

                match md.authorize(&token).await {
                    Ok(()) => (),
                    Err(e) => return Outcome::Failure((Status::Unauthorized, e)),
                };

                match create_orbit(
                    md.id(),
                    config.database.path.clone(),
                    &auth_data,
                    relay,
                    keys,
                )
                .await
                {
                    Ok(Some(orbit)) => Outcome::Success(Self(orbit)),
                    Ok(None) => {
                        return Outcome::Failure((
                            Status::Conflict,
                            anyhow!("Orbit already exists"),
                        ))
                    }
                    Err(e) => Outcome::Failure((Status::InternalServerError, e)),
                }
            }
            _ => Outcome::Failure((
                Status::BadRequest,
                anyhow!("Token action not matching endpoint"),
            )),
        }
    }
}
