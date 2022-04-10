#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate tokio;

use anyhow::Result;
use rocket::{fairing::AdHoc, figment::Figment, http::Header, tokio::fs, Build, Rocket};

pub mod allow_list;
pub mod auth;
pub mod capabilities;
pub mod cas;
pub mod codec;
pub mod config;
pub mod indexes;
pub mod ipfs;
pub mod manifest;
pub mod orbit;
pub mod relay;
pub mod resource;
pub mod routes;
pub mod s3;
pub mod siwe;
pub mod storage;
pub mod transport;
pub mod tz;
pub mod zcap;

use libp2p::{
    identity::{ed25519::Keypair as Ed25519Keypair, Keypair},
    PeerId,
};
use relay::RelayNode;
use routes::core::{
    batch_put_content, cors, delete_content, get_content, get_content_no_auth, list_content,
    list_content_no_auth, open_host_key, open_orbit_allowlist, open_orbit_authz, put_content,
    relay_addr,
};
use routes::s3 as s3_routes;
use std::{collections::HashMap, sync::RwLock};

#[get("/healthz")]
pub fn healthcheck() {}

pub fn tracing_try_init() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();
}

pub async fn app(config: &Figment) -> Result<Rocket<Build>> {
    let kepler_config = config.extract::<config::Config>()?;

    storage::KV::healthcheck(kepler_config.storage.indexes.clone()).await?;
    storage::StorageUtils::new(kepler_config.storage.blocks.clone())
        .healthcheck()
        .await?;

    let kp = storage::StorageUtils::relay_key_pair(kepler_config.storage.blocks).await?;
    let relay_node = RelayNode::new(kepler_config.relay.port, Keypair::Ed25519(kp)).await?;

    let mut routes = routes![
        healthcheck,
        put_content,
        batch_put_content,
        delete_content,
        open_orbit_allowlist,
        open_orbit_authz,
        cors,
        s3_routes::put_content,
        s3_routes::delete_content,
        relay_addr,
        open_host_key
    ];

    if kepler_config.orbits.public {
        let mut no_auth = routes![
            get_content_no_auth,
            list_content_no_auth,
            s3_routes::get_content_no_auth,
            s3_routes::get_metadata_no_auth,
            s3_routes::list_content_no_auth,
        ];
        routes.append(&mut no_auth);
    } else {
        let mut auth = routes![
            get_content,
            list_content,
            s3_routes::get_content,
            s3_routes::get_metadata,
            s3_routes::list_content,
        ];
        routes.append(&mut auth);
    }

    Ok(rocket::custom(config)
        .mount("/", routes)
        .attach(AdHoc::config::<config::Config>())
        .attach(AdHoc::on_response("CORS", |_, resp| {
            Box::pin(async move {
                resp.set_header(Header::new("Access-Control-Allow-Origin", "*"));
                resp.set_header(Header::new(
                    "Access-Control-Allow-Methods",
                    "POST, PUT, GET, OPTIONS, DELETE",
                ));
                resp.set_header(Header::new("Access-Control-Allow-Headers", "*"));
                resp.set_header(Header::new("Access-Control-Allow-Credentials", "true"));
            })
        }))
        .manage(relay_node)
        .manage(RwLock::new(HashMap::<PeerId, Ed25519Keypair>::new())))
}
