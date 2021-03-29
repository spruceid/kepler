use crate::tz;
use anyhow::Result;
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
    TezosSignature(tz::TZAuth),
}

impl<'a, 'r> FromRequest<'a, 'r> for AuthToken {
    type Error = anyhow::Error;

    fn from_request(request: &'a Request<'r>) -> Outcome<Self, Self::Error> {
        Outcome::Success(Self::None)
    }
}
