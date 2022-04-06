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

use crate::auth::{
    DelAuthWrapper, GetAuthWrapper, ListAuthWrapper, MetadataAuthWrapper, PutAuthWrapper,
};
use crate::cas::CidWrap;
use crate::config;
use crate::orbit::load_orbit;
use crate::relay::RelayNode;
use crate::routes::DotPathBuf;
use crate::s3::{ObjectBuilder, ObjectReader};
use crate::capabilities::AuthRef;
use std::collections::BTreeMap;

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

pub struct S3Response(ObjectReader, pub Metadata);

impl S3Response {
    pub fn new(md: Metadata, reader: ObjectReader) -> Self {
        Self(reader, md)
    }
}

impl<'r> Responder<'r, 'static> for S3Response {
    fn respond_to(self, r: &'r Request<'_>) -> response::Result<'static> {
        Ok(Response::build_from(self.1.respond_to(r)?)
            // must ensure that Metadata::respond_to does not set the body of the response
            .streamed_body(self.0)
            .finalize())
    }
}

#[get("/<orbit_id>/s3")]
pub async fn list_content_no_auth(
    orbit_id: CidWrap,
    config: &State<config::Config>,
    relay: &State<RelayNode>,
) -> Result<Json<Vec<String>>, (Status, String)> {
    let orbit = match load_orbit(orbit_id.0, config, (relay.id, relay.internal())).await {
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
    orbit: MetadataAuthWrapper,
    key: DotPathBuf,
) -> Result<Option<Metadata>, (Status, String)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed".into())),
    };
    match orbit.0.service.get(k).await {
        Ok(Some(content)) => Ok(Some(Metadata(content.metadata))),
        Err(e) => Err((Status::InternalServerError, e.to_string())),
        Ok(None) => Ok(None),
    }
}

#[head("/<orbit_id>/s3/<key..>")]
pub async fn get_metadata_no_auth(
    orbit_id: CidWrap,
    key: DotPathBuf,
    config: &State<config::Config>,
    relay: &State<RelayNode>,
) -> Result<Option<Metadata>, (Status, String)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed".into())),
    };
    let orbit = match load_orbit(orbit_id.0, config, (relay.id, relay.internal())).await {
        Ok(Some(o)) => o,
        Ok(None) => return Err((Status::NotFound, anyhow!("Orbit not found").to_string())),
        Err(e) => return Err((Status::InternalServerError, e.to_string())),
    };
    match orbit.service.get(k).await {
        Ok(Some(content)) => Ok(Some(Metadata(content.metadata))),
        Err(e) => Err((Status::InternalServerError, e.to_string())),
        Ok(None) => Ok(None),
    }
}

#[get("/<_orbit_id>/s3/<key..>", rank = 8)]
pub async fn get_content(
    _orbit_id: CidWrap,
    orbit: GetAuthWrapper,
    key: DotPathBuf,
) -> Result<Option<S3Response>, (Status, String)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed".into())),
    };
    match orbit.0.service.read(k).await {
        Ok(Some((md, r))) => Ok(Some(S3Response::new(Metadata(md), r))),
        _ => Ok(None),
    }
}

#[get("/<orbit_id>/s3/<key..>", rank = 8)]
pub async fn get_content_no_auth(
    orbit_id: CidWrap,
    key: DotPathBuf,
    config: &State<config::Config>,
    relay: &State<RelayNode>,
) -> Result<Option<S3Response>, (Status, String)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed".into())),
    };
    let orbit = match load_orbit(orbit_id.0, config, (relay.id, relay.internal())).await {
        Ok(Some(o)) => o,
        Ok(None) => return Err((Status::NotFound, anyhow!("Orbit not found").to_string())),
        Err(e) => return Err((Status::InternalServerError, e.to_string())),
    };

    match orbit.service.read(k).await {
        Ok(Some((md, r))) => Ok(Some(S3Response::new(Metadata(md), r))),
        _ => Ok(None),
    }
}

#[put("/<_orbit_id>/s3/<key..>", data = "<data>")]
pub async fn put_content(
    _orbit_id: CidWrap,
    orbit: PutAuthWrapper,
    key: DotPathBuf,
    md: Metadata,
    data: Data<'_>,
) -> Result<(), (Status, String)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed".into())),
    };

    let rm: [([u8; 0], _, _); 0] = [];

    orbit
        .0
        .service
        .write(
            [(
                ObjectBuilder::new(k.as_bytes().to_vec(), md.0, orbit.1),
                data.open(1u8.gigabytes()),
            )],
            rm,
        )
        .await
        .map_err(|e| (Status::InternalServerError, e.to_string()))?;
    Ok(())
}

#[delete("/<_orbit_id>/s3/<key..>")]
pub async fn delete_content(
    _orbit_id: CidWrap,
    orbit: DelAuthWrapper,
    key: DotPathBuf,
) -> Result<(), (Status, &'static str)> {
    let k = match key.to_str() {
        Some(k) => k,
        _ => return Err((Status::BadRequest, "Key parsing failed")),
    };
    let add: Vec<(&[u8], Cid)> = vec![];
    Ok(orbit
        .0
        .service
        .index(add, vec![(k, None, orbit.1)])
        .await
        .map_err(|_| (Status::InternalServerError, "Failed to delete content"))?)
}
