use anyhow::Result;
use rocket::{
    data::{Data, ToByteUnit},
    http::Status,
    request::{FromRequest, Outcome, Request},
    State,
};

use crate::cas::{CidWrap, ContentAddressedStorage};
use crate::config;
use crate::orbit::load_orbit;
use crate::relay::RelayNode;
use crate::s3::ObjectBuilder;
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

#[get("/s3/<orbit_id>/<key>")]
pub async fn get(
    orbit_id: CidWrap,
    key: String,
    config: &State<config::Config>,
    relay: &State<RelayNode>,
) -> Result<Option<Vec<u8>>, (Status, String)> {
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
    let s3_obj = match orbit.service.get(key) {
        Ok(Some(content)) => content,
        _ => return Ok(None),
    };
    match orbit.get(&s3_obj.value).await {
        Ok(Some(content)) => Ok(Some(content.to_vec())),
        Ok(None) => Ok(None),
        Err(_) => Ok(None),
    }
}

#[put("/s3/<orbit_id>/<key>", data = "<data>")]
pub async fn put(
    orbit_id: CidWrap,
    key: String,
    md: Metadata,
    data: Data<'_>,
    config: &State<config::Config>,
    relay: &State<RelayNode>,
) -> Result<(), (Status, String)> {
    let orbit = match load_orbit(
        orbit_id.0,
        config.database.path.clone(),
        (relay.id, relay.internal()),
    )
    .await
    {
        Ok(Some(o)) => o,
        Ok(None) => return Err((Status::NotFound, "Orbit not found".to_string())),
        Err(e) => return Err((Status::InternalServerError, e.to_string())),
    };
    orbit
        .service
        .write(
            vec![(
                ObjectBuilder::new(key.as_bytes().to_vec(), md.0),
                data.open(1u8.megabytes())
                    .into_bytes()
                    .await
                    .map_err(|e| (Status::BadRequest, anyhow!(e).to_string()))?
                    .to_vec(),
            )],
            vec![],
        )
        .map_err(|e| (Status::InternalServerError, e.to_string()))?;
    Ok(())
}
