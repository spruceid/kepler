use kepler_core::util::{DelegationInfo, InvocationInfo, RevocationInfo};
use kepler_lib::authorization::{
    EncodingError, HeaderEncode, KeplerDelegation, KeplerInvocation, KeplerRevocation,
};
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use std::convert::TryFrom;

#[derive(thiserror::Error, Debug)]
pub enum FromReqErr<T> {
    #[error(transparent)]
    Encoding(#[from] EncodingError),
    #[error(transparent)]
    TryFrom(T),
}

macro_rules! impl_fromreq {
    ($type:ident, $inter:ident, $name:tt) => {
        #[rocket::async_trait]
        impl<'r> FromRequest<'r> for $type {
            type Error = FromReqErr<<$type as TryFrom<$inter>>::Error>;
            async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
                match request.headers().get_one($name).map(|e| {
                    $type::try_from(<$inter as HeaderEncode>::decode(e)?)
                        .map_err(FromReqErr::TryFrom)
                }) {
                    Some(Ok(item)) => Outcome::Success(item),
                    Some(Err(e)) => Outcome::Failure((Status::Unauthorized, e)),
                    None => Outcome::Forward(()),
                }
            }
        }
    };
}

impl_fromreq!(DelegationInfo, KeplerDelegation, "Authorization");
impl_fromreq!(InvocationInfo, KeplerInvocation, "Authorization");
impl_fromreq!(RevocationInfo, KeplerRevocation, "Authorization");

#[cfg(test)]
mod test {
    use super::*;
    use kepler_lib::{
        resolver::DID_METHODS,
        ssi::{
            did::{Document, Source},
            did_resolve::DIDResolver,
            jwk::{Algorithm, JWK},
            jws::Header,
            ucan::{Capability, Payload},
            vc::NumericDate,
        },
    };

    async fn gen(
        iss: &JWK,
        aud: String,
        caps: Vec<Capability>,
        exp: f64,
        prf: Vec<Cid>,
    ) -> (Document, Thing) {
        let did = DID_METHODS
            .generate(&Source::KeyAndPattern(iss, "key"))
            .unwrap();
        (
            DID_METHODS
                .resolve(&did, &Default::default())
                .await
                .1
                .unwrap(),
            gen_ucan((iss, did), aud, caps, exp, prf).await,
        )
    }
    async fn gen_ucan(
        iss: (&JWK, String),
        audience: String,
        attenuation: Vec<Capability>,
        exp: f64,
        proof: Vec<Cid>,
    ) -> Thing {
        let p = Payload {
            issuer: iss.1,
            audience,
            attenuation,
            proof,
            nonce: None,
            not_before: None,
            facts: None,
            expiration: NumericDate::try_from_seconds(exp).unwrap(),
        }
        .sign(Algorithm::EdDSA, iss.0)
        .unwrap();
        Thing {
            token: p.encode().unwrap(),
            payload: p.payload,
            header: p.header,
        }
    }

    #[derive(serde::Serialize)]
    struct Thing {
        pub token: String,
        pub payload: Payload,
        pub header: Header,
    }
    #[test]
    async fn basic() -> Result<()> {
        Ok(())
    }
}
