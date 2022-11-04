#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
#[cfg(test)]
#[macro_use]
extern crate tokio;

use anyhow::Result;
use kepler_lib::libipld::{block::Block as OBlock, store::DefaultParams};
use rocket::{fairing::AdHoc, figment::Figment, http::Header, Build, Rocket};

pub mod allow_list;
pub mod auth_guards;
pub mod authorization;
pub mod capabilities;
pub mod cas;
pub mod config;
pub mod indexes;
pub mod kv;
pub mod manifest;
pub mod orbit;
pub mod p2p;
pub mod prometheus;
pub mod relay;
pub mod routes;
pub mod storage;
mod tracing;
pub mod transport;

use config::{BlockStorage, Config};
use libp2p::{
    identity::{ed25519::Keypair as Ed25519Keypair, Keypair},
    PeerId,
};
use orbit::ProviderUtils;
use relay::RelayNode;
use routes::{delegate, invoke, open_host_key, relay_addr, util_routes::*};
use std::{collections::HashMap, sync::RwLock};
use storage::{
    either::Either,
    file_system::{FileSystemConfig, FileSystemStore},
    s3::{S3BlockConfig, S3BlockStore},
};

pub type Block = OBlock<DefaultParams>;
pub type BlockStores = Either<S3BlockStore, FileSystemStore>;
pub type BlockConfig = Either<S3BlockConfig, FileSystemConfig>;

impl Default for BlockConfig {
    fn default() -> Self {
        Self::B(FileSystemConfig::default())
    }
}

impl From<BlockStorage> for BlockConfig {
    fn from(c: BlockStorage) -> Self {
        match c {
            BlockStorage::S3(s) => Self::A(s),
            BlockStorage::Local(l) => Self::B(l),
        }
    }
}

impl From<BlockConfig> for BlockStorage {
    fn from(c: BlockConfig) -> Self {
        match c {
            BlockConfig::A(a) => Self::S3(a),
            BlockConfig::B(b) => Self::Local(b),
        }
    }
}

pub async fn app(config: &Figment) -> Result<Rocket<Build>> {
    let kepler_config: Config = config.extract::<Config>()?;

    tracing::tracing_try_init(&kepler_config.log);

    storage::KV::healthcheck(kepler_config.storage.indexes.clone()).await?;
    let kp = kepler_config.storage.blocks.relay_key_pair().await?;

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
