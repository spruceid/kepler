use crate::{
    codec::PutContent,
    orbit::SimpleOrbit,
    tz::{verify, TZAuth},
    Orbits,
};
use anyhow::Result;
use core::str::FromStr;
use libipld::cid::Cid;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};

pub enum ContentAction {
    Put(Vec<PutContent>),
    Get(Vec<Cid>),
    Del(Vec<Cid>),
}

pub struct CreateOrbit {
    pkh: String,
    action: Option<ContentAction>,
}

pub enum Action {
    Content {
        orbit_id: Cid,
        action: ContentAction,
    },
    Orbit(CreateOrbit),
}

pub trait AuthorizationToken: Sized {
    const header_key: &'static str;
    fn extract<'a, T: Iterator<Item = &'a str>>(auth_data: T) -> Result<Self>;
    fn action(&self) -> &Action;
}

#[rocket::async_trait]
pub trait AuthorizationPolicy {
    type Token: AuthorizationToken;
    async fn authorize(&self, auth_token: &Self::Token) -> Result<ContentAction>;
}

pub enum AuthToken {
    None,
    TezosSignature(TZAuth),
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthToken {
    type Error = anyhow::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let auth_headers: Vec<&'r str> = request.headers().get("Authentication").collect();
        match auth_headers.first() {
            Some(auth_header) => match TZAuth::from_str(auth_header) {
                Ok(tza) => match verify(&tza) {
                    Ok(()) => Outcome::Success(Self::TezosSignature(tza)),
                    Err(e) => Outcome::Failure((Status::Unauthorized, e)),
                },
                Err(e) => Outcome::Failure((Status::Unauthorized, e)),
            },
            None => Outcome::Success(Self::None),
        }
    }
}
