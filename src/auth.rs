use crate::tz::{verify, TZAuth};
use anyhow::Result;
use core::str::FromStr;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};

pub trait AuthorizationToken: Sized {
    fn extract<'a, T: Iterator<Item = &'a str>>(auth_data: T) -> Result<Self>;
    fn header_key() -> &'static str;
}

pub trait AuthrorizationStrategy: Sized {
    type Token: AuthorizationToken;
    type Action;
    fn authorize(&self, auth_token: Self::Token) -> Result<Self::Action>;
}

pub enum AuthToken {
    None,
    TezosSignature(TZAuth),
}

impl<'a, 'r> FromRequest<'a, 'r> for AuthToken {
    type Error = anyhow::Error;

    fn from_request(request: &'a Request<'r>) -> Outcome<Self, Self::Error> {
        let auth_headers: Vec<&'a str> = request
            .headers()
            .get("Authentication")
            .inspect(|s| println!("{}", s))
            .collect();
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
