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

#[derive(PartialEq)]
pub struct DummyToken;

pub struct DummyAuthorization;

pub enum AuthToken {
    None,
    TezosSignature(String),
}

impl AuthorizationToken for DummyToken {
    fn extract<'a, T: Iterator<Item = &'a str>>(auth_data: T) -> Result<Self> {
        Ok(Self)
    }
    fn header_key() -> &'static str {
        ""
    }
}

impl AuthrorizationStrategy for DummyAuthorization {
    type Token = DummyToken;
    type Action = ();
    fn authorize(&self, auth_token: Self::Token) -> Result<Self::Action> {
        Ok(())
    }
}

impl<'a, 'r, T> FromRequest<'a, 'r> for AuthToken {
    type Error = anyhow::Error;

    fn from_request(request: &'a Request<'r>) -> Outcome<Self, Self::Error> {
        match T::extract(request.headers().get(T::header_key())) {
            Ok(auth) => Outcome::Success(Self(auth)),
            Err(e) => Outcome::Failure((Status::Unauthorized, anyhow!(format!("{}", e)))),
        }
    }
}
