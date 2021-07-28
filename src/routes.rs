use anyhow::{Error, Result};
use rocket::response::Debug;
use rocket::{
    data::{Data, ToByteUnit},
    http::Status,
    serde::json::Json,
    State,
};
use std::path::PathBuf;

use crate::auth::{
    CreateAuthWrapper, DelAuthWrapper, GetAuthWrapper, ListAuthWrapper, PutAuthWrapper,
};
use crate::cas::{CidWrap, ContentAddressedStorage};
use crate::codec::SupportedCodecs;
use crate::config;
use crate::orbit::{create_orbit, load_orbit, Orbit};

// TODO need to check for every relevant endpoint that the orbit ID in the URL matches the one in the auth token
async fn uri_listing(orbit: Orbit) -> Result<Json<Vec<String>>, (Status, String)> {
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

#[get("/<_orbit_id>")]
pub async fn list_content(
    _orbit_id: CidWrap,
    orbit: ListAuthWrapper,
    config: &State<config::Config>,
) -> Result<Json<Vec<String>>, (Status, String)> {
    uri_listing(orbit.0).await
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

#[get("/<_orbit_id>/<hash>")]
pub async fn get_content(
    _orbit_id: CidWrap,
    hash: CidWrap,
    orbit: GetAuthWrapper,
) -> Result<Option<Vec<u8>>, Debug<Error>> {
    match orbit.0.get(&hash.0).await {
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

#[post("/<_orbit_id>", data = "<data>", rank = 1)]
pub async fn put_content(
    _orbit_id: CidWrap,
    data: Data,
    codec: SupportedCodecs,
    orbit: GetAuthWrapper,
    config: &State<config::Config>,
) -> Result<String, (Status, String)> {
    match orbit
        .0
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
        Ok(cid) => Ok(orbit.0.make_uri(&cid).map_err(|_| {
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

#[post("/create", rank = 2)]
pub async fn create_orbit_(_orbit: CreateAuthWrapper) -> Result<(), Debug<Error>> {
    Ok(())
}

#[options("/<_s..>")]
pub async fn cors(_s: PathBuf) -> () {
    ()
}
