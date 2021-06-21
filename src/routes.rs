use anyhow::{Error, Result};
use rocket::http::HeaderMap;
use rocket::request::FromRequest;
use rocket::request::Outcome;
use rocket::response::Debug;
use rocket::Request;
use rocket::{
    data::{Data, ToByteUnit},
    form::Form,
    http::Status,
    serde::json::Json,
    State,
};
use std::path::PathBuf;

use crate::auth::AuthorizationToken;
use crate::cas::{CidWrap, ContentAddressedStorage};
use crate::codec::{PutContent, SupportedCodecs};
use crate::config;
use crate::orbit::{create_orbit, load_orbit, Orbit, SimpleOrbit};
use crate::zcap::{ZCAPDelegation, ZCAPInvocation};

pub struct DelegationHeader(String);
#[rocket::async_trait]
impl<'r> FromRequest<'r> for DelegationHeader {
    type Error = anyhow::Error;
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match req.headers().get_one("Delegation") {
            Some(h) => Outcome::Success(DelegationHeader(h.to_string())),
            None => Outcome::Forward(()),
        }
    }
}
pub struct InvocationHeader(String);
#[rocket::async_trait]
impl<'r> FromRequest<'r> for InvocationHeader {
    type Error = anyhow::Error;
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match req.headers().get_one("Invocation") {
            Some(h) => Outcome::Success(InvocationHeader(h.to_string())),
            None => Outcome::Forward(()),
        }
    }
}

// TODO need to check for every relevant endpoint that the orbit ID in the URL matches the one in the auth token

async fn uri_listing(orbit: SimpleOrbit) -> Result<Json<Vec<String>>, (Status, String)> {
    orbit
        .list()
        .await
        .map_err(|_| {
            (
                Status::InternalServerError,
                "Failed to list Orbit contents".to_string(),
            )
        })
        .and_then(|l| {
            l.into_iter()
                .map(|c| {
                    orbit.make_uri(&c).map_err(|_| {
                        (
                            Status::InternalServerError,
                            "Failed to serialize CID".to_string(),
                        )
                    })
                })
                .collect::<Result<Vec<String>, (Status, String)>>()
                .map(|v| Json(v))
        })
}

#[get("/<orbit_id>")]
pub async fn list_content(
    orbit_id: CidWrap,
    config: &State<config::Config>,
) -> Result<Json<Vec<String>>, (Status, String)> {
    let orbit = match load_orbit(orbit_id.0, config.database.path.clone()).await {
        // Ok(Some(o)) => o,
        // Ok(None) => return Err((Status::NotFound, anyhow!("No Orbit found").to_string())),
        Ok(o) => o,
        Err(e) => return Err((Status::InternalServerError, e.to_string())),
    };
    // TODO check ZCAP
    uri_listing(orbit).await
}

#[get("/<orbit_id>", rank = 2)]
pub async fn list_content_no_auth(
    orbit_id: CidWrap,
    config: &State<config::Config>,
) -> Result<Json<Vec<String>>, (Status, String)> {
    let orbit = match load_orbit(orbit_id.0, config.database.path.clone()).await {
        Ok(Some(o)) => o,
        Ok(None) => return Err((Status::NotFound, anyhow!("Orbit not found").to_string())),
        Err(e) => return Err((Status::InternalServerError, e.to_string())),
    };
    uri_listing(orbit).await
}

#[get("/<orbit_id>/<hash>")]
pub async fn get_content(
    orbit_id: CidWrap,
    hash: CidWrap,
    delegation: DelegationHeader,
    invocation: InvocationHeader,
    config: &State<config::Config>,
) -> Result<Option<Vec<u8>>, Debug<Error>> {
    let orbit = match load_orbit(orbit_id.0, config.database.path.clone()).await {
        // Ok(Some(o)) => o,
        // Ok(None) => return Err((Status::NotFound, anyhow!("No Orbit found").to_string())),
        Ok(o) => o,
        Err(e) => Err(anyhow!(e.to_string()))?,
    };
    let zcap_delegation =
        ZCAPDelegation(serde_json::from_str(&delegation.0).map_err(|e| anyhow!(e.to_string()))?);
    let zcap_verification = ZCAPInvocation::extract(&invocation.0)?
        .0
        .verify(None, &did_pkh::DIDPKH, &zcap_delegation.0)
        .await;
    if zcap_verification.errors.len() > 0 {
        Err(anyhow!(
            "ZCAP delegation verification errors: {:?}",
            zcap_verification.errors
        ))?
    }
    let user = match zcap_delegation.0.invoker {
        Some(did) => match did {
            ssi::vc::URI::String(d) => d,
        },
        None => Err(anyhow!("No invoker"))?,
    };
    let user_pkh = match user.strip_prefix("did:pkh:eth:") {
        Some(pkh) => pkh,
        None => Err(anyhow!("Invoker not did:pkh:eth"))?,
    };
    if !(true) {
        Err(anyhow!("You do not own "))?
    }

    // TODO check issuer is owner of orbit
    // let issuer =
    // let orbit_uri = format!("eth;address={};domain={};index={}", "0x6Da01670d8fc844e736095918bbE11fE8D564163", domain, index);
    match orbit.get(&hash.0).await {
        Ok(Some(content)) => Ok(Some(content.to_vec())),
        Ok(None) => Ok(None),
        Err(_) => Ok(None),
    }
}

#[get("/<orbit_id>/<hash>", rank = 2)]
pub async fn get_content_no_auth(
    orbit_id: CidWrap,
    hash: CidWrap,
    config: &State<config::Config>,
) -> Result<Option<Vec<u8>>, (Status, String)> {
    let orbit = match load_orbit(orbit_id.0, config.database.path.clone()).await {
        Ok(Some(o)) => o,
        Ok(None) => return Err((Status::NotFound, anyhow!("Orbit not found").to_string())),
        Err(e) => return Err((Status::InternalServerError, e.to_string())),
    };
    match orbit.get(&hash.0).await {
        Ok(Some(content)) => Ok(Some(content.to_vec())),
        Ok(None) => Ok(None),
        Err(_) => Ok(None),
    }
}

#[post("/<orbit_id>", data = "<data>", rank = 1)]
pub async fn put_content(
    orbit_id: CidWrap,
    data: Data,
    codec: SupportedCodecs,
    config: &State<config::Config>,
) -> Result<String, (Status, String)> {
    let orbit = match load_orbit(orbit_id.0, config.database.path.clone()).await {
        // Ok(Some(o)) => o,
        // Ok(None) => return Err((Status::NotFound, anyhow!("No Orbit found").to_string())),
        Ok(o) => o,
        Err(e) => return Err((Status::InternalServerError, e.to_string())),
    };
    match orbit
        .put(
            &data
                .open(1u8.megabytes())
                .into_bytes()
                .await
                .map_err(|_| (Status::BadRequest, "Failed to stream content".to_string()))?,
            codec,
        )
        .await
    {
        Ok(cid) => Ok(orbit.make_uri(&cid).map_err(|_| {
            (
                Status::InternalServerError,
                "Failed to generate URI".to_string(),
            )
        })?),
        Err(_) => Err((
            Status::InternalServerError,
            "Failed to store content".to_string(),
        )),
    }
}

#[post("/<orbit_id>/create", rank = 2)]
pub async fn create_orbit_(
    orbit_id: CidWrap,
    config: &State<config::Config>,
) -> Result<(), Debug<Error>> {
    create_orbit(orbit_id.0, config.database.path.clone(), vec![]).await?;
    Ok(())
}

#[options("/<_s..>")]
pub async fn cors(_s: PathBuf) -> () {
    ()
}
