use crate::cas::CidWrap;
use crate::config;
use crate::orbit::{create_orbit, get_metadata, load_orbit, AuthTokens, Orbit};
use crate::relay::RelayNode;
use anyhow::Result;
use ipfs_embed::{Keypair, Multiaddr, PeerId};
use libipld::cid::Cid;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::RwLock};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Put(Vec<String>),
    Get(Vec<String>),
    Del(Vec<String>),
    Create {
        parameters: String,
        content: Vec<String>,
    },
    List,
}

pub trait AuthorizationToken {
    fn action(&self) -> &Action;
    fn target_orbit(&self) -> &Cid;
}

#[rocket::async_trait]
pub trait AuthorizationPolicy<T> {
    async fn authorize(&self, auth_token: &T) -> Result<()>;
}

pub struct PutAuthWrapper(pub Orbit);
pub struct GetAuthWrapper(pub Orbit);
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
                            Status::InternalServerError,
                            anyhow!("Could not parse orbit"),
                        ));
                    }
                };
                match (token.action(), &oid == token.target_orbit()) {
                    (_, false) => Outcome::Failure((
                        Status::BadRequest,
                        anyhow!("Token target orbit not matching endpoint"),
                    )),
                    (Action::$method { .. }, true) => {
                        let orbit = match load_orbit(
                            *token.target_orbit(),
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

impl_fromreq!(PutAuthWrapper, Put);
impl_fromreq!(GetAuthWrapper, Get);
impl_fromreq!(DelAuthWrapper, Del);
impl_fromreq!(ListAuthWrapper, List);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for CreateAuthWrapper {
    type Error = anyhow::Error;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let (auth_data, token, config, relay) = match extract_info(req).await {
            Ok(i) => i,
            Err(o) => return o,
        };
        let keys = match req.rocket().state::<RwLock<HashMap<PeerId, Keypair>>>() {
            Some(k) => k,
            _ => {
                return Outcome::Failure((
                    Status::InternalServerError,
                    anyhow!("Could not retrieve open key set"),
                ));
            }
        };

        match &token.action() {
            // Create actions dont have an existing orbit to authorize against, it's a node policy
            // TODO have policy config, for now just be very permissive :shrug:
            Action::Create { parameters, .. } => {
                let md = match get_metadata(token.target_orbit(), parameters, &config.chains).await
                {
                    Ok(md) => md,
                    Err(e) => return Outcome::Failure((Status::Unauthorized, e)),
                };

                match md.authorize(&token).await {
                    Ok(()) => (),
                    Err(e) => return Outcome::Failure((Status::Unauthorized, e)),
                };

                match create_orbit(&md, config.database.path.clone(), &auth_data, relay, keys).await
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
