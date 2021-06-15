use anyhow::Result;
use rocket::{
    data::{Data, ToByteUnit},
    form::Form,
    http::Status,
    serde::json::Json,
    State,
};
use std::path::PathBuf;

use crate::cas::{CidWrap, ContentAddressedStorage};
use crate::codec::{PutContent, SupportedCodecs};
use crate::config;
use crate::orbit::{Orbit, SimpleOrbit};

// TODO need to check for every relevant endpoint that the orbit ID in the URL matches the one in the auth token

#[get("/<_orbit_id>")]
pub async fn list_content(
    _orbit_id: CidWrap,
    orbit: SimpleOrbit,
    // Don't need it as we're using req.rocket().state ?
    // config: &State<config::Config>,
) -> Result<Json<Vec<String>>, (Status, &'static str)> {
    orbit
        .list()
        .await
        .map_err(|_| (Status::InternalServerError, "Failed to list Orbit contents"))
        .and_then(|l| {
            l.into_iter()
                .map(|c| {
                    orbit
                        .make_uri(&c)
                        .map_err(|_| (Status::InternalServerError, "Failed to serialize CID"))
                })
                .collect::<Result<Vec<String>, (Status, &'static str)>>()
                .map(|v| Json(v))
        })
}

#[get("/<_orbit_id>", rank = 2)]
pub async fn list_content_no_auth(
    _orbit_id: CidWrap,
    orbit: SimpleOrbit,
    config: &State<config::Config>,
) -> Result<Json<Vec<String>>, (Status, &'static str)> {
    orbit
        .list()
        .await
        .map_err(|_| (Status::InternalServerError, "Failed to list Orbit contents"))
        .and_then(|l| {
            l.into_iter()
                .map(|c| {
                    orbit
                        .make_uri(&c)
                        .map_err(|_| (Status::InternalServerError, "Failed to serialize CID"))
                })
                .collect::<Result<Vec<String>, (Status, &'static str)>>()
                .map(|v| Json(v))
        })
}

#[get("/<_orbit_id>/<hash>")]
pub async fn get_content(
    _orbit_id: CidWrap,
    orbit: SimpleOrbit,
    hash: CidWrap,
) -> Result<Option<Vec<u8>>, (Status, &'static str)> {
    match orbit.get(&hash.0).await {
        Ok(Some(content)) => Ok(Some(content.to_vec())),
        Ok(None) => Ok(None),
        Err(_) => Ok(None),
    }
}

#[get("/<_orbit_id>/<hash>", rank = 2)]
pub async fn get_content_no_auth(
    _orbit_id: CidWrap,
    orbit: SimpleOrbit,
    hash: CidWrap,
    config: &State<config::Config>,
) -> Result<Option<Vec<u8>>, (Status, &'static str)> {
    match orbit.get(&hash.0).await {
        Ok(Some(content)) => Ok(Some(content.to_vec())),
        Ok(None) => Ok(None),
        Err(_) => Ok(None),
    }
}

#[post("/<_orbit_id>", format = "multipart/form-data", data = "<batch>")]
pub async fn batch_put_content(
    _orbit_id: CidWrap,
    orbit: SimpleOrbit,
    batch: Form<Vec<PutContent>>,
) -> Result<String, (Status, &'static str)> {
    let mut uris = Vec::<String>::new();
    for content in batch.into_inner().into_iter() {
        uris.push(
            orbit
                .put(&content.content, content.codec)
                .await
                .map_or("".into(), |cid| {
                    orbit.make_uri(&cid).map_or("".into(), |s| s)
                }),
        );
    }
    Ok(uris.join("\n"))
}

#[post("/<_orbit_id>", data = "<data>", rank = 2)]
pub async fn put_content(
    _orbit_id: CidWrap,
    orbit: SimpleOrbit,
    data: Data,
    codec: SupportedCodecs,
) -> Result<String, (Status, &'static str)> {
    match orbit
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
            .make_uri(&cid)
            .map_err(|_| (Status::InternalServerError, "Failed to generate URI"))?),
        Err(_) => Err((Status::InternalServerError, "Failed to store content")),
    }
}

#[post("/", format = "multipart/form-data", data = "<batch>")]
pub async fn batch_put_create(
    orbit: SimpleOrbit,
    batch: Form<Vec<PutContent>>,
) -> Result<String, (Status, &'static str)> {
    let mut uris = Vec::<String>::new();
    for content in batch.into_inner().into_iter() {
        uris.push(
            orbit
                .put(&content.content, content.codec)
                .await
                .map_or("".into(), |cid| {
                    orbit.make_uri(&cid).map_or("".into(), |s| s)
                }),
        );
    }
    Ok(uris.join("\n"))
}

#[post("/", data = "<data>", rank = 2)]
pub async fn put_create(
    orbit: SimpleOrbit,
    data: Data,
    codec: SupportedCodecs,
) -> Result<String, (Status, &'static str)> {
    let uri = orbit
        .make_uri(
            &orbit
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
    orbit: SimpleOrbit,
    hash: CidWrap,
) -> Result<(), (Status, &'static str)> {
    Ok(orbit
        .delete(&hash.0)
        .await
        .map_err(|_| (Status::InternalServerError, "Failed to delete content"))?)
}

#[options("/<_s..>")]
pub async fn cors(_s: PathBuf) -> () {
    ()
}
