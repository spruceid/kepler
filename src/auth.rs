use crate::config;
use crate::orbit::{create_orbit, load_orbit, verify_oid, Orbit};
use crate::tz::{TezosAuthorizationString, TezosBasicAuthorization};
use anyhow::Result;
use libipld::cid::Cid;
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
};
use ssi::did::DIDURL;

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
    List {
        orbit_id: Cid,
    },
}

pub trait AuthorizationToken {
    const HEADER_KEY: &'static str;
    fn extract(auth_data: &str) -> Result<Self>;
    fn action(&self) -> &Action;
}

#[rocket::async_trait]
pub trait AuthorizationPolicy {
    type Token: AuthorizationToken;
    async fn authorize<'a>(&self, auth_token: &'a Self::Token) -> Result<&'a Action>;
}

// TODO some APIs prefer to return 404 when the authentication fails to avoid leaking information about content
#[rocket::async_trait]
impl<'r, T> FromRequest<'r> for T
where
    T: Orbit,
{
    type Error = anyhow::Error;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        // TODO need to identify auth method from the headers
        let auth_data = match req.headers().get_one(TezosAuthorizationString::HEADER_KEY) {
            Some(a) => a,
            None => {
                return Outcome::Forward(());
            }
        };
        info_!("Headers: {}", auth_data);

        let config = match req.rocket().state::<config::Config>() {
            Some(c) => c,
            None => {
                return Outcome::Failure((
                    Status::InternalServerError,
                    anyhow!("Could not retrieve config"),
                ))
            }
        };

        // get token from headers
        let token = match TezosAuthorizationString::extract(auth_data) {
            Ok(t) => t,
            Err(e) => return Outcome::Failure((Status::Unauthorized, e)),
        };
        match token.action() {
            // content actions have the same authz process
            Action::Put { orbit_id, .. }
            | Action::Get { orbit_id, .. }
            | Action::List { orbit_id }
            | Action::Del { orbit_id, .. } => {
                let orbit = match load_orbit(config.database.path, orbit_id) {
                    Some(o) => o,
                    None => {
                        return Outcome::Failure((Status::Unauthorized, anyhow!("No Orbit found")))
                    }
                };
                match orbit.auth().authorize(&token).await {
                    Ok(_) => Outcome::Success(orbit),
                    Err(e) => Outcome::Failure((Status::Unauthorized, e)),
                }
            }
            // Create actions dont have an existing orbit to authorize against, it's a node policy
            // TODO have policy config, for now just be very permissive :shrug:
            Action::Create {
                orbit_id,
                parameters,
                ..
            } => match TezosBasicAuthorization.authorize(&token).await {
                Ok(_) => {
                    if let Err(e) = verify_oid(orbit_id, &token.pkh, parameters) {
                        Outcome::Failure((Status::BadRequest, "Incorrect Orbit ID"))
                    }
                    let vm = DIDURL {
                        did: format!("did:pkh:tz:{}", &token.pkh),
                        fragment: Some("TezosMethod2021".to_string()),
                        ..Default::default()
                    };
                    match create_orbit(
                        orbit_id,
                        config.database.path,
                        TezosBasicAuthorization,
                        vec![vm],
                        auth_data.as_bytes(),
                    )
                    .await
                    {
                        Ok(orbit) => Outcome::Success(orbit),
                        Err(e) => Outcome::Failure((Status::InternalServerError, e)),
                    }
                }
                Err(e) => Outcome::Failure((Status::Unauthorized, e)),
            },
        }
    }
}
