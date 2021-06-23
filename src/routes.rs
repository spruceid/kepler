use anyhow::Result;
use rocket::{
    data::{Data, ToByteUnit},
    form::Form,
    http::Status,
    serde::json::Json,
    State,
};
use std::path::PathBuf;

use crate::auth::{
    CreateAuthWrapper, DelAuthWrapper, GetAuthWrapper, ListAuthWrapper, PutAuthWrapper,
};
use crate::cas::{CidWrap, ContentAddressedStorage};
use crate::codec::{PutContent, SupportedCodecs};
use crate::config;
use crate::orbit::{load_orbit, Orbit, SimpleOrbit};

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

#[get("/<_orbit_id>")]
pub async fn list_content(
    _orbit_id: CidWrap,
    orbit: ListAuthWrapper,
    // Don't need it as we're using req.rocket().state ?
    // config: &State<config::Config>,
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
    orbit: GetAuthWrapper,
    hash: CidWrap,
) -> Result<Option<Vec<u8>>, (Status, &'static str)> {
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

#[post("/<_orbit_id>", format = "multipart/form-data", data = "<batch>")]
pub async fn batch_put_content(
    _orbit_id: CidWrap,
    orbit: PutAuthWrapper,
    batch: Form<Vec<PutContent>>,
) -> Result<String, (Status, &'static str)> {
    let mut uris = Vec::<String>::new();
    for content in batch.into_inner().into_iter() {
        uris.push(
            orbit
                .0
                .put(&content.content, content.codec)
                .await
                .map_or("".into(), |cid| {
                    orbit.0.make_uri(&cid).map_or("".into(), |s| s)
                }),
        );
    }
    Ok(uris.join("\n"))
}

#[post("/<_orbit_id>", data = "<data>", rank = 2)]
pub async fn put_content(
    _orbit_id: CidWrap,
    orbit: PutAuthWrapper,
    data: Data,
    codec: SupportedCodecs,
) -> Result<String, (Status, &'static str)> {
    match orbit
        .0
        .put(
            &data
                .open(1u8.megabytes())
                .into_bytes()
                .await
                .map_err(|_| (Status::BadRequest, "Failed to stream content"))?,
            codec,
        )
        .await
    {
        Ok(cid) => Ok(orbit
            .0
            .make_uri(&cid)
            .map_err(|_| (Status::InternalServerError, "Failed to generate URI"))?),
        Err(_) => Err((Status::InternalServerError, "Failed to store content")),
    }
}

#[post("/", format = "multipart/form-data", data = "<batch>")]
pub async fn batch_put_create(
    orbit: CreateAuthWrapper,
    batch: Form<Vec<PutContent>>,
) -> Result<String, (Status, &'static str)> {
    let mut uris = Vec::<String>::new();
    for content in batch.into_inner().into_iter() {
        uris.push(
            orbit
                .0
                .put(&content.content, content.codec)
                .await
                .map_or("".into(), |cid| {
                    orbit.0.make_uri(&cid).map_or("".into(), |s| s)
                }),
        );
    }
    Ok(uris.join("\n"))
}

#[post("/", data = "<data>", rank = 2)]
pub async fn put_create(
    orbit: CreateAuthWrapper,
    data: Data,
    codec: SupportedCodecs,
) -> Result<String, (Status, &'static str)> {
    let uri = orbit
        .0
        .make_uri(
            &orbit
                .0
                .put(
                    &data
                        .open(1u8.megabytes())
                        .into_bytes()
                        .await
                        .map_err(|_| (Status::BadRequest, "Failed to stream content"))?,
                    codec,
                )
                .await
                .map_err(|_| (Status::InternalServerError, "Failed to store content"))?,
        )
        .map_err(|_| (Status::InternalServerError, "Failed to generate URI"))?;
    Ok(uri)
}

#[delete("/<_orbit_id>/<hash>")]
pub async fn delete_content(
    _orbit_id: CidWrap,
    orbit: DelAuthWrapper,
    hash: CidWrap,
) -> Result<(), (Status, &'static str)> {
    Ok(orbit
        .0
        .delete(&hash.0)
        .await
        .map_err(|_| (Status::InternalServerError, "Failed to delete content"))?)
}

#[options("/<_s..>")]
pub async fn cors(_s: PathBuf) -> () {
    ()
}
