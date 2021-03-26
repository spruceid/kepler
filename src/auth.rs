use anyhow::Result;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};

pub trait AuthrorizationStrategy: Sized {
    fn authorize<'a, T: Iterator<Item = &'a str>>(auth_data: T) -> Result<Self>;
}

pub struct Authorization<S: AuthrorizationStrategy>(S);

#[derive(PartialEq)]
pub struct DummyAuth;

impl AuthrorizationStrategy for DummyAuth {
    fn authorize<'a, T: Iterator<Item = &'a str>>(auth_data: T) -> Result<Self> {
        Ok(Self)
    }
}

impl<'a, 'r, T> FromRequest<'a, 'r> for Authorization<T>
where
    T: AuthrorizationStrategy,
{
    type Error = anyhow::Error;

    fn from_request(request: &'a Request<'r>) -> Outcome<Self, Self::Error> {
        match T::authorize(request.headers().get("Authorization")) {
            Ok(auth) => Outcome::Success(Self(auth)),
            Err(e) => Outcome::Failure((Status::Unauthorized, anyhow!(format!("{}", e)))),
        }
    }
}
