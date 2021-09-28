use anyhow::{Error, Result};
use rocket::response::Debug;
use rocket::{
    data::{Data, ToByteUnit},
    form::Form,
    http::Status,
    serde::json::Json,
    State,
};
use std::path::PathBuf;
use libp2p::multiaddr::Protocol;

use crate::allow_list::OrbitAllowList;
use crate::auth::{
    CreateAuthWrapper, DelAuthWrapper, GetAuthWrapper, ListAuthWrapper, PutAuthWrapper,
};
use crate::cas::{CidWrap, ContentAddressedStorage};
use crate::codec::{PutContent, SupportedCodecs};
use crate::config;
use crate::orbit::{create_orbit, load_orbit, verify_oid, AuthTypes, Orbit};
use crate::relay::RelayNode;

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
) -> Result<Json<Vec<String>>, (Status, String)> {
    uri_listing(orbit.0).await
}

#[get("/<orbit_id>", rank = 2)]
pub async fn list_content_no_auth(
    orbit_id: CidWrap,
    config: &State<config::Config>,
    relay: &State<RelayNode>
) -> Result<Json<Vec<String>>, (Status, String)> {
    let orbit = match load_orbit(orbit_id.0, config.database.path.clone(), (relay.id, relay.internal())).await {
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
    relay: &State<RelayNode>
) -> Result<Option<Vec<u8>>, (Status, String)> {
    let orbit = match load_orbit(orbit_id.0, config.database.path.clone(), (relay.id, relay.internal())).await {
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

#[put("/<_orbit_id>", data = "<data>")]
pub async fn put_content(
    _orbit_id: CidWrap,
    data: Data<'_>,
    codec: SupportedCodecs,
    orbit: PutAuthWrapper,
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

#[put(
    "/<_orbit_id>",
    format = "multipart/form-data",
    data = "<batch>",
    rank = 2
)]
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

#[post("/<_orbit_id>", format = "text/plain", data = "<_params_str>")]
pub async fn open_orbit_authz(
    _orbit_id: CidWrap,
    _params_str: &str,
    _authz: CreateAuthWrapper,
) -> Result<(), (Status, &'static str)> {
    // create auth success, return OK
    Ok(())
}

#[post(
    "/al/<orbit_id>",
    format = "text/plain",
    data = "<params_str>",
    rank = 2
)]
pub async fn open_orbit_allowlist(
    orbit_id: CidWrap,
    params_str: &str,
    config: &State<config::Config>,
    relay: &State<RelayNode>
) -> Result<(), (Status, &'static str)> {
    // no auth token, use allowlist
    match (
        verify_oid(&orbit_id.0, params_str),
        config.orbits.allowlist.as_ref(),
    ) {
        (_, None) => Err((Status::InternalServerError, "Allowlist Not Configured")),
        (Ok(_), Some(list)) => match list.is_allowed(&orbit_id.0).await {
            Ok(controllers) => {
                create_orbit(
                    orbit_id.0,
                    config.database.path.clone(),
                    controllers,
                    &[],
                    AuthTypes::ZCAP,
                    (relay.id, relay.internal())
                )
                .await
                .map_err(|_| (Status::InternalServerError, "Failed to create Orbit"))?;
                Ok(())
            }
            _ => Err((Status::Unauthorized, "Orbit not allowed")),
        },
        (Err(_), _) => Err((Status::BadRequest, "Invalid Orbit Params")),
    }
}

#[options("/<_s..>")]
pub async fn cors(_s: PathBuf) -> () {
    ()
}

#[get("/relay")]
pub fn relay_addr(
    relay: &State<RelayNode>
) -> String {
    relay.external()
         .with(Protocol::P2p(relay.id.into()))
         .with(Protocol::P2pCircuit)
         .to_string()
}
