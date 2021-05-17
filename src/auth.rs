use crate::{
    orbit::{Orbit, SimpleOrbit},
    OrbitCollection, Orbits,
};
use anyhow::Result;
use libipld::cid::Cid;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};

#[derive(Debug)]
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
}

pub trait AuthorizationToken: Sized {
    type Policy: AuthorizationPolicy<Token = Self> + Send + Sync;
    const HEADER_KEY: &'static str;
    fn extract(auth_data: &str) -> Result<Self>;
    fn action(&self) -> &Action;
}

#[rocket::async_trait]
pub trait AuthorizationPolicy {
    type Token: AuthorizationToken<Policy = Self>;
    async fn authorize<'a>(&self, auth_token: &'a Self::Token) -> Result<&'a Action>;
}

pub struct AuthWrapper<T: AuthorizationToken>(pub T);

#[rocket::async_trait]
impl<'r, T: 'static + AuthorizationToken + Send + Sync> FromRequest<'r> for AuthWrapper<T> {
    type Error = anyhow::Error;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let auth_data = match req.headers().get_one(T::HEADER_KEY) {
            Some(a) => a,
            None => {
                return Outcome::Failure((Status::Unauthorized, anyhow!("No Authorization Header")))
            }
        };

        // get token from headers
        let token = match T::extract(auth_data) {
            Ok(t) => t,
            Err(e) => return Outcome::Failure((Status::Unauthorized, e)),
        };
        // get orbits state object
        let orbits = match req.rocket().state::<Orbits<SimpleOrbit<T::Policy>>>() {
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
            Action::Put { orbit_id, .. }
            | Action::Get { orbit_id, .. }
            | Action::Del { orbit_id, .. } => {
                let read_orbits = orbits.orbits().await;
                let orbit = match read_orbits.get(orbit_id) {
                    Some(o) => o,
                    None => {
                        return Outcome::Failure((
                            Status::Unauthorized,
                            anyhow!("No authorization policy found"),
                        ))
                    }
                };
                match orbit.auth().authorize(&token).await {
                    Ok(_) => Outcome::Success(AuthWrapper(token)),
                    Err(e) => Outcome::Failure((Status::Unauthorized, e)),
                }
            }
            // Create actions dont have an existing orbit to authorize against, it's a node policy
            // TODO have policy config, for now just be very permissive :shrug:
            Action::Create { .. } => match req.rocket().state::<T::Policy>() {
                Some(auth) => match auth.authorize(&token).await {
                    Ok(_) => Outcome::Success(AuthWrapper(token)),
                    Err(e) => Outcome::Failure((Status::Unauthorized, e)),
                },
                None => Outcome::Success(AuthWrapper(token)),
            },
        }
    }
}
