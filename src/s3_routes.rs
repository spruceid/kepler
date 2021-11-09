use anyhow::Result;
use libipld::Cid;
use rocket::{
    data::{Data, ToByteUnit},
    http::{Header, Status},
    request::{FromRequest, Outcome, Request},
    response::{self, Responder, Response},
    serde::json::Json,
    State,
};

use crate::auth::{DelAuthWrapper, GetAuthWrapper, ListAuthWrapper, PutAuthWrapper};
use crate::cas::{CidWrap, ContentAddressedStorage};
use crate::config;
use crate::orbit::load_orbit;
use crate::relay::RelayNode;
use crate::s3::ObjectBuilder;
use std::{collections::BTreeMap, path::PathBuf};

pub struct Metadata(pub BTreeMap<String, String>);

#[async_trait]
impl<'r> FromRequest<'r> for Metadata {
    type Error = anyhow::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let md: BTreeMap<String, String> = request
            .headers()
            .iter()
            .map(|h| (h.name.into_string(), h.value.to_string()))
            .collect();
        Outcome::Success(Metadata(md))
    }
}

impl<'r> Responder<'r, 'static> for Metadata {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let mut r = Response::build();
        for (k, v) in self.0 {
            if k != "content-length" {
                r.header(Header::new(k, v));
            }
        }
        Ok(r.finalize())
    }
}

pub struct S3Response(pub Vec<u8>, pub Metadata);

impl<'r> Responder<'r, 'static> for S3Response {
    fn respond_to(self, r: &'r Request<'_>) -> response::Result<'static> {
        Ok(Response::build_from(self.0.respond_to(r)?)
            // must ensure that Metadata::respond_to does not set the body of the response
            .merge(self.1.respond_to(r)?)
            .finalize())
    }
}

#[get("/<orbit_id>/s3", rank = 8)]
pub async fn list_content_no_auth(
    orbit_id: CidWrap,
    config: &State<config::Config>,
    relay: &State<RelayNode>,
) -> Result<Json<Vec<String>>, (Status, String)> {
    let orbit = match load_orbit(
        orbit_id.0,
        config.database.path.clone(),
        (relay.id, relay.internal()),
    )
    .await
    {
        Ok(Some(o)) => o,
        Ok(None) => return Err((Status::NotFound, anyhow!("Orbit not found").to_string())),
        Err(e) => return Err((Status::InternalServerError, e.to_string())),
    };
    Ok(Json(
        orbit
            .service
            .list()
            .filter_map(|r| {
                // filter out any non-utf8 keys
                r.map(|v| std::str::from_utf8(v.as_ref()).ok().map(|s| s.to_string()))
                    .transpose()
            })
            .collect::<Result<Vec<String>>>()
            .map_err(|e| (Status::InternalServerError, e.to_string()))?,
    ))
}

#[get("/<_orbit_id>/s3")]
pub async fn list_content(
    _orbit_id: CidWrap,
    orbit: ListAuthWrapper,
) -> Result<Json<Vec<String>>, (Status, String)> {
    Ok(Json(
        orbit
            .0
            .service
            .list()
            .filter_map(|r| {
                // filter out any non-utf8 keys
                r.map(|v| std::str::from_utf8(v.as_ref()).ok().map(|s| s.to_string()))
                    .transpose()
            })
            .collect::<Result<Vec<String>>>()
            .map_err(|e| (Status::InternalServerError, e.to_string()))?,
    ))
}

#[head("/<_orbit_id>/s3/<key..>")]
pub async fn get_metadata(
    _orbit_id: CidWrap,
    orbit: GetAuthWrapper,
    key: PathBuf,
) -> Result<Option<Metadata>, (Status, String)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed".into())),
    };
    match orbit.0.service.get(k) {
        Ok(Some(content)) => Ok(Some(Metadata(content.metadata))),
        Err(e) => Err((Status::InternalServerError, e.to_string())),
        Ok(None) => Ok(None),
    }
}

#[head("/<orbit_id>/s3/<key..>")]
pub async fn get_metadata_no_auth(
    orbit_id: CidWrap,
    key: PathBuf,
    config: &State<config::Config>,
    relay: &State<RelayNode>,
) -> Result<Option<Metadata>, (Status, String)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed".into())),
    };
    let orbit = match load_orbit(
        orbit_id.0,
        config.database.path.clone(),
        (relay.id, relay.internal()),
    )
    .await
    {
        Ok(Some(o)) => o,
        Ok(None) => return Err((Status::NotFound, anyhow!("Orbit not found").to_string())),
        Err(e) => return Err((Status::InternalServerError, e.to_string())),
    };
    match orbit.service.get(k) {
        Ok(Some(content)) => Ok(Some(Metadata(content.metadata))),
        Err(e) => Err((Status::InternalServerError, e.to_string())),
        Ok(None) => Ok(None),
    }
}

#[get("/<_orbit_id>/s3/<key..>")]
pub async fn get_content(
    _orbit_id: CidWrap,
    orbit: GetAuthWrapper,
    key: PathBuf,
) -> Result<Option<S3Response>, (Status, String)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed".into())),
    };
    let s3_obj = match orbit.0.service.get(k) {
        Ok(Some(content)) => content,
        _ => return Ok(None),
    };
    match orbit.0.get(&s3_obj.value).await {
        Ok(Some(content)) => Ok(Some(S3Response(
            content.to_vec(),
            Metadata(s3_obj.metadata),
        ))),
        Ok(None) => Ok(None),
        Err(_) => Ok(None),
    }
}

#[get("/<orbit_id>/s3/<key..>")]
pub async fn get_content_no_auth(
    orbit_id: CidWrap,
    key: PathBuf,
    config: &State<config::Config>,
    relay: &State<RelayNode>,
) -> Result<Option<S3Response>, (Status, String)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed".into())),
    };
    let orbit = match load_orbit(
        orbit_id.0,
        config.database.path.clone(),
        (relay.id, relay.internal()),
    )
    .await
    {
        Ok(Some(o)) => o,
        Ok(None) => return Err((Status::NotFound, anyhow!("Orbit not found").to_string())),
        Err(e) => return Err((Status::InternalServerError, e.to_string())),
    };
    let s3_obj = match orbit.service.get(k) {
        Ok(Some(content)) => content,
        _ => return Ok(None),
    };
    match orbit.get(&s3_obj.value).await {
        Ok(Some(content)) => Ok(Some(S3Response(
            content.to_vec(),
            Metadata(s3_obj.metadata),
        ))),
        Ok(None) => Ok(None),
        Err(_) => Ok(None),
    }
}

#[put("/<_orbit_id>/s3/<key..>", data = "<data>")]
pub async fn put_content(
    _orbit_id: CidWrap,
    orbit: PutAuthWrapper,
    key: PathBuf,
    md: Metadata,
    data: Data<'_>,
) -> Result<(), (Status, String)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed".into())),
    };
    let rm: Vec<(Vec<u8>, Option<(u64, Cid)>)> = vec![];

    orbit
        .0
        .service
        .write(
            vec![(ObjectBuilder::new(k.as_bytes().to_vec(), md.0), data.open(1u8.gigabytes()))],
            rm
        ).await
        .map_err(|e| (Status::InternalServerError, e.to_string()))?;
    Ok(())
}

#[delete("/<_orbit_id>/s3/<key..>")]
pub async fn delete_content(
    _orbit_id: CidWrap,
    orbit: DelAuthWrapper,
    key: PathBuf,
) -> Result<(), (Status, &'static str)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed".into())),
    };
    let add: Vec<(&[u8], Cid)> = vec![];
    Ok(orbit
        .0
        .service
        .index(add, vec![(k, None)])
        .map_err(|_| (Status::InternalServerError, "Failed to delete content"))?)
}
