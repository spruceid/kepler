use crate::{
    codec::{PutContent, SupportedCodecs},
    orbit::SimpleOrbit,
    tz::{verify, TZAuth, TezosBasicAuthorization},
    Orbits,
};
use anyhow::Result;
use core::str::FromStr;
use libipld::{
    cid::Cid,
    codec::Codec,
    multihash::{Code, Digest},
};
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};

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
        pkh: String,
        salt: String,
        put: Option<Vec<Cid>>,
    },
}

pub trait AuthorizationToken: Sized {
    const header_key: &'static str;
    fn extract<'a, T: Iterator<Item = &'a str>>(auth_data: T) -> Result<Self>;
    fn action(&self) -> &Action;
}

#[rocket::async_trait]
pub trait AuthorizationPolicy {
    type Token: AuthorizationToken;
    async fn authorize<'a>(&self, auth_token: &'a Self::Token) -> Result<&'a Action>;
}

pub enum AuthToken {
    None,
    TezosSignature(TZAuth),
}

pub struct AuthWrapper<T: AuthorizationToken>(T);

#[rocket::async_trait]
impl<'r, T: AuthorizationToken> FromRequest<'r> for AuthWrapper<T> {
    type Error = anyhow::Error;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        // get token from headers
        let token = match T::extract(req.headers().get(T::header_key)) {
            Ok(t) => t,
            Err(e) => return Outcome::Failure((Status::Unauthorized, e)),
        };
        // get orbits state object
        let orbits = match req.rocket().state::<Orbits<SimpleOrbit>>() {
            Some(orbits) => orbits,
            None => {
                return Outcome::Failure((
                    Status::Unauthorized,
                    anyhow!("No authorization policy found"),
                ))
            }
        };

        match token.action() {
            // content actions have the same authz process
            Action::Put { orbit_id, content }
            | Action::Get { orbit_id, content }
            | Action::Del { orbit_id, content } => {
                let orbit = match orbits.orbit(orbit_id) {
                    Some(o) => o,
                    None => {
                        return Outcome::Failure((
                            Status::Unauthorized,
                            anyhow!("No authorization policy found"),
                        ))
                    }
                };
                match orbit.auth().authorize(&token) {
                    Ok(_) => Outcome::Success(AuthWrapper(token)),
                    Err(e) => Outcome::Failure((Status::Unauthorized, e)),
                }
            }
            // Create actions dont have an existing orbit to authorize against, it's a node policy
            // TODO have policy config, for now just be very permissive :shrug:
            Action::Create { pkh, salt, put } => {
                match req.rocket().state::<TezosBasicAuthorization>() {
                    Some(auth) => match auth.authorize(&token) {
                        Ok(_) => Outcome::Success(AuthWrapper(token)),
                        Err(e) => Outcome::Failure((Status::Unauthorized, e)),
                    },
                    None => Outcome::Success(AuthWrapper(token)),
                }
            }
        }
    }
}
