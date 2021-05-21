use anyhow::Result;
use rocket::{
    data::{Data, ToByteUnit},
    fairing::AdHoc,
    figment::Figment,
    form::Form,
    http::{Header, Status},
    response::Stream,
    Build, Rocket, State,
};
use ssi::did::DIDURL;
use std::{io::Cursor, path::PathBuf};

use crate::auth::{Action, AuthWrapper, AuthorizationToken};
use crate::cas::{CidWrap, ContentAddressedStorage};
use crate::codec::{PutContent, SupportedCodecs};
use crate::config::Config;
use crate::orbit::{
    create_orbit, load_orbits, verify_oid_v0, Orbit, OrbitCollection, Orbits, SimpleOrbit,
};
use crate::tz::{TezosAuthorizationString, TezosBasicAuthorization};

#[get("/<orbit_id>/<hash>")]
async fn get_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    _auth: Option<AuthWrapper<TezosAuthorizationString>>,
) -> Result<Option<Stream<Cursor<Vec<u8>>>>, (Status, &'static str)> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or_else(|| (Status::NotFound, "No Orbit Found"))?;
    match orbit.get(&hash.0).await {
        Ok(Some(content)) => Ok(Some(Stream::chunked(Cursor::new(content), 1024))),
        Ok(None) => Ok(None),
        Err(_) => Ok(None),
    }
}

#[post("/<orbit_id>", format = "multipart/form-data", data = "<batch>")]
async fn batch_put_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    batch: Form<Vec<PutContent>>,
    _auth: AuthWrapper<TezosAuthorizationString>,
) -> Result<String, (Status, &'static str)> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or_else(|| (Status::NotFound, "No Orbit Found"))?;
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

#[post("/<orbit_id>", data = "<data>", rank = 2)]
async fn put_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    data: Data,
    codec: SupportedCodecs,
    _auth: AuthWrapper<TezosAuthorizationString>,
) -> Result<String, (Status, &'static str)> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or_else(|| (Status::NotFound, "No Orbit Found"))?;
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
async fn batch_put_create(
    // TODO find a good way to not restrict all orbits to the same Type
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    batch: Form<Vec<PutContent>>,
    auth: AuthWrapper<TezosAuthorizationString>,
) -> Result<String, (Status, &'static str)> {
    match auth.0.action() {
        Action::Create {
            orbit_id,
            parameters,
            ..
        } => {
            verify_oid_v0(orbit_id, &auth.0.pkh, parameters)
                .map_err(|_| (Status::BadRequest, "Incorrect Orbit ID"))?;

            let vm = DIDURL {
                did: format!("did:pkh:tz:{}", &auth.0.pkh),
                fragment: Some("TezosMethod2021".to_string()),
                ..Default::default()
            };

            let orbit = create_orbit(
                *orbit_id,
                &orbits.base_path,
                TezosBasicAuthorization,
                vec![vm],
                auth.0.to_string().as_bytes(),
            )
            .await
            .map_err(|_| (Status::Conflict, "Orbit Already Exists"))?;

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
            orbits.add(orbit).await;
            Ok(uris.join("\n"))
        }
        _ => Err((Status::Unauthorized, "Incorrectly Authorized Action")),
    }
}

#[post("/", data = "<data>", rank = 2)]
async fn put_create(
    // TODO find a good way to not restrict all orbits to the same Type
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    data: Data,
    codec: SupportedCodecs,
    auth: AuthWrapper<TezosAuthorizationString>,
) -> Result<String, (Status, &'static str)> {
    match auth.0.action() {
        Action::Create {
            orbit_id,
            parameters,
            ..
        } => {
            verify_oid_v0(orbit_id, &auth.0.pkh, parameters)
                .map_err(|_| (Status::BadRequest, "Incorrect Orbit ID"))?;

            let vm = DIDURL {
                did: format!("did:pkh:tz:{}", &auth.0.pkh),
                fragment: Some("TezosMethod2021".to_string()),
                ..Default::default()
            };

            let orbit = create_orbit(
                *orbit_id,
                &orbits.base_path,
                TezosBasicAuthorization,
                vec![vm],
                auth.0.to_string().as_bytes(),
            )
            .await
            .map_err(|_| (Status::Conflict, "Orbit Already Exists"))?;

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

            orbits.add(orbit).await;

            Ok(uri)
        }
        _ => Err((Status::Unauthorized, "Incorrectly Authorized Action")),
    }
}

#[delete("/<orbit_id>/<hash>")]
async fn delete_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    _auth: AuthWrapper<TezosAuthorizationString>,
) -> Result<(), (Status, &'static str)> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or_else(|| (Status::NotFound, "No Orbit Found"))?;
    Ok(orbit
        .delete(&hash.0)
        .await
        .map_err(|_| (Status::InternalServerError, "Failed to delete content"))?)
}

#[options("/<_s..>")]
async fn cors(_s: PathBuf) -> () {
    ()
}

pub async fn app(config: Figment) -> Result<Rocket<Build>> {
    let kepler_config = config.extract::<Config>()?;

    // ensure KEPLER_DATABASE_PATH exists
    if !kepler_config.database.path.is_dir() {
        return Err(anyhow!(
            "KEPLER_DATABASE_PATH does not exist or is not a directory: {:?}",
            kepler_config.database.path.to_str()
        ));
    }

    Ok(rocket::custom(config)
        .manage(load_orbits(kepler_config.database.path).await?)
        .manage(TezosBasicAuthorization)
        .mount(
            "/",
            routes![
                get_content,
                put_content,
                batch_put_content,
                delete_content,
                put_create,
                batch_put_create,
                cors
            ],
        )
        .attach(AdHoc::on_response("CORS", |_, resp| {
            Box::pin(async move {
                resp.set_header(Header::new("Access-Control-Allow-Origin", "*"));
                resp.set_header(Header::new(
                    "Access-Control-Allow-Methods",
                    "POST, GET, OPTIONS, DELETE",
                ));
                resp.set_header(Header::new("Access-Control-Allow-Headers", "*"));
                resp.set_header(Header::new("Access-Control-Allow-Credentials", "true"));
            })
        })))
}
