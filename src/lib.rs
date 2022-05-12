#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate tokio;

use anyhow::Result;
use rocket::{fairing::AdHoc, figment::Figment, http::Header, Build, Rocket};

pub mod allow_list;
pub mod auth;
pub mod capabilities;
pub mod cas;
pub mod codec;
pub mod config;
pub mod indexes;
pub mod ipfs;
pub mod kv;
pub mod manifest;
pub mod orbit;
pub mod prometheus;
pub mod relay;
pub mod resource;
pub mod routes;
pub mod siwe;
pub mod storage;
mod tracing;
pub mod transport;
pub mod zcap;

use ::tracing::subscriber::set_global_default;
use libp2p::{
    identity::{ed25519::Keypair as Ed25519Keypair, Keypair},
    PeerId,
};
use relay::RelayNode;
use routes::{cors, delegate, invoke, open_host_key, relay_addr};
use std::{collections::HashMap, sync::RwLock};
use tracing_log::LogTracer;
use tracing_subscriber::{layer::SubscriberExt, Layer, Registry};

#[get("/healthz")]
pub fn healthcheck() {}

pub fn tracing_try_init(config: &config::Logging) {
    LogTracer::init().unwrap();
    let env_filter = tracing_subscriber::EnvFilter::from_default_env();
    let subscriber = tracing_subscriber::fmt::layer();
    let log = match config.format {
        config::LoggingFormat::Text => subscriber.boxed(),
        config::LoggingFormat::Json => subscriber.json().boxed(),
    };
    let telemetry = if config.tracing.enabled {
        let tracer = opentelemetry_jaeger::new_pipeline()
            .with_service_name("kepler")
            .install_batch(opentelemetry::runtime::Tokio)
            .unwrap();
        let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
        Some(telemetry)
    } else {
        None
    };
    let collector = Registry::default()
        .with(env_filter)
        .with(log)
        .with(telemetry);
    set_global_default(collector).unwrap();
}

pub async fn app(config: &Figment) -> Result<Rocket<Build>> {
    let kepler_config = config.extract::<config::Config>()?;

    tracing_try_init(&kepler_config.log);

    storage::KV::healthcheck(kepler_config.storage.indexes.clone()).await?;
    storage::StorageUtils::new(kepler_config.storage.blocks.clone())
        .healthcheck()
        .await?;

    let kp = storage::StorageUtils::relay_key_pair(kepler_config.storage.blocks).await?;
    let relay_node = RelayNode::new(kepler_config.relay.port, Keypair::Ed25519(kp)).await?;

    let routes = routes![
        healthcheck,
        cors,
        relay_addr,
        open_host_key,
        invoke,
        delegate,
    ];

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
        .attach(tracing::TracingFairing {
            header_name: kepler_config.log.tracing.traceheader,
        })
        .manage(relay_node)
        .manage(RwLock::new(HashMap::<PeerId, Ed25519Keypair>::new())))
}
