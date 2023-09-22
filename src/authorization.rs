use kepler_core::events::SerializedEvent;
use kepler_lib::authorization::{Delegation, EncodingError, Revocation};
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};

pub struct AuthHeaderGetter<T>(pub SerializedEvent<T>);

macro_rules! impl_fromreq {
    ($type:ident, $name:tt) => {
        #[rocket::async_trait]
        impl<'r> FromRequest<'r> for AuthHeaderGetter<$type> {
            type Error = EncodingError;
            async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
                match request
                    .headers()
                    .get_one($name)
                    .map(SerializedEvent::<$type>::from_header_ser)
                {
                    Some(Ok(e)) => Outcome::Success(AuthHeaderGetter(e)),
                    Some(Err(e)) => Outcome::Failure((Status::Unauthorized, e)),
                    None => Outcome::Forward(()),
                }
            }
        }
    };
}

impl_fromreq!(Delegation, "Authorization");
// currently delegations and invocations are really the same type
// impl_fromreq!(Invocation, "Authorization");
impl_fromreq!(Revocation, "Authorization");
