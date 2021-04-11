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

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthToken {
    type Error = anyhow::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let auth_headers: Vec<&'r str> = request.headers().get("Authentication").collect();
        match auth_headers.first() {
            Some(auth_header) => match TZAuth::from_str(auth_header) {
                Ok(tza) => match verify(&tza).await {
                    Ok(()) => Outcome::Success(Self::TezosSignature(tza)),
                    Err(e) => Outcome::Failure((Status::Unauthorized, e)),
                },
                Err(e) => Outcome::Failure((Status::Unauthorized, e)),
            },
            None => Outcome::Success(Self::None),
        }
    }
}
