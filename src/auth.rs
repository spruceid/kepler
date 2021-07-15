use crate::config;
use crate::orbit::{
    create_orbit, get_oid_matrix_params, load_orbit, verify_oid, AuthTokens, Orbit, SimpleOrbit,
};
use crate::tz::{TezosAuthorizationString, TezosBasicAuthorization};
use anyhow::Result;
use ipfs_embed::Keypair;
use libipld::cid::Cid;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use ssi::did::DIDURL;

#[derive(Debug, Clone)]
pub enum Action {
    Put {
        orbit_id: Cid,
        content: Vec<Cid>,
    },
    Get {
        orbit_id: Cid,
        content: Vec<Cid>,
    },
    Del {
        orbit_id: Cid,
        content: Vec<Cid>,
    },
    Create {
        orbit_id: Cid,
        parameters: String,
        content: Vec<Cid>,
    },
    List {
        orbit_id: Cid,
    },
}

pub trait AuthorizationToken {
    fn extract(auth_data: &str) -> Result<Self>
    where
        Self: Sized;
    fn action(&self) -> &Action;
}

#[rocket::async_trait]
pub trait AuthorizationPolicy {
    type Token: AuthorizationToken;
    async fn authorize<'a>(&self, auth_token: &'a Self::Token) -> Result<()>;
}

pub struct PutAuthWrapper(pub SimpleOrbit);
pub struct GetAuthWrapper(pub SimpleOrbit);
pub struct DelAuthWrapper(pub SimpleOrbit);
pub struct CreateAuthWrapper(pub SimpleOrbit);
pub struct ListAuthWrapper(pub SimpleOrbit);

fn extract_info<'a, T>(
    req: &'a Request,
) -> Result<(Vec<u8>, AuthTokens, config::Config, &'a Keypair), Outcome<T, anyhow::Error>> {
    // TODO need to identify auth method from the headers
    let auth_data = match req.headers().get_one("Authorization") {
        Some(a) => a,
        None => {
            return Err(Outcome::Forward(()));
        }
    };
    info_!("Headers: {}", auth_data);
    let config = match req.rocket().state::<config::Config>() {
        Some(c) => c,
        None => {
            return Err(Outcome::Failure((
                Status::InternalServerError,
                anyhow!("Could not retrieve config"),
            )));
        }
    };
    let kp = match req.rocket().state::<Keypair>() {
        Some(kp) => kp,
        None => {
            return Err(Outcome::Failure((
                Status::InternalServerError,
                anyhow!("Could not retrieve key pair"),
            )))
        }
    };
    match TezosAuthorizationString::extract(auth_data) {
        Ok(token) => Ok((
            auth_data.as_bytes().to_vec(),
            AuthTokens::Tezos(token),
            config.clone(),
            kp,
        )),
        Err(e) => Err(Outcome::Failure((Status::Unauthorized, e))),
    }
}

// TODO some APIs prefer to return 404 when the authentication fails to avoid leaking information about content

macro_rules! impl_fromreq {
    ($type:ident, $method:tt) => {
        #[rocket::async_trait]
        impl<'r> FromRequest<'r> for $type {
            type Error = anyhow::Error;

            async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
                let (_, token, config, kp) = match extract_info(req) {
                    Ok(i) => i,
                    Err(o) => return o,
                };
                match token.action() {
                    Action::$method { orbit_id, .. } => {
                        let orbit = match load_orbit(*orbit_id, config.database.path.clone(), kp)
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
                        match orbit.auth().authorize(token).await {
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
        let (auth_data, token, config, kp) = match extract_info(req) {
            Ok(i) => i,
            Err(o) => return o,
        };
        // TODO remove clone, or refactor the order of validations/actions
        match token.clone().action() {
            // Create actions dont have an existing orbit to authorize against, it's a node policy
            // TODO have policy config, for now just be very permissive :shrug:
            Action::Create {
                orbit_id,
                parameters,
                ..
            } => match token {
                AuthTokens::Tezos(token_tz) => {
                    if let Err(_) = verify_oid(orbit_id, parameters) {
                        return Outcome::Failure((
                            Status::BadRequest,
                            anyhow!("Incorrect Orbit ID"),
                        ));
                    }

                    match create_orbit(
                        *orbit_id,
                        config.database.path.clone(),
                        &auth_data,
                        &parameters,
                        kp,
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
            },
            _ => Outcome::Failure((
                Status::BadRequest,
                anyhow!("Token action not matching endpoint"),
            )),
        }
    }
}
