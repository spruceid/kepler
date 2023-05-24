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
pub mod cas;
pub mod config;
pub mod manifest;
pub mod orbit;
pub mod prometheus;
pub mod relay;
pub mod routes;
pub mod storage;
mod tracing;
pub mod transport;

use config::{BlockStorage, Config, StagingStorage};
use kepler_core::{
    migrations::Migrator,
    sea_orm::Database,
    sea_orm_migration::MigratorTrait,
    storage::{either::Either, memory::MemoryStaging},
};
use libp2p::{
    identity::{ed25519::Keypair as Ed25519Keypair, Keypair},
    PeerId,
};
use orbit::ProviderUtils;
use relay::RelayNode;
use routes::{delegate, invoke, open_host_key, relay_addr, util_routes::*};
use std::{collections::HashMap, sync::RwLock};
use storage::{
    file_system::{FileSystemConfig, FileSystemStore, TempFileSystemStage},
    s3::{S3BlockConfig, S3BlockStore},
};

pub type Block = OBlock<DefaultParams>;
pub type BlockStores = Either<S3BlockStore, FileSystemStore>;
pub type BlockConfig = Either<S3BlockConfig, FileSystemConfig>;
pub type BlockStage = Either<TempFileSystemStage, MemoryStaging>;

impl Into<BlockConfig> for BlockStorage {
    fn into(self) -> BlockConfig {
        match self {
            Self::S3(s) => BlockConfig::A(s),
            Self::Local(l) => BlockConfig::B(l),
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

impl From<StagingStorage> for BlockStage {
    fn from(c: StagingStorage) -> Self {
        match c {
            StagingStorage::Memory => Self::B(MemoryStaging::default()),
            StagingStorage::FileSystem => Self::A(TempFileSystemStage::default()),
        }
    }
}

impl From<BlockStage> for StagingStorage {
    fn from(c: BlockStage) -> Self {
        match c {
            BlockStage::B(_) => Self::Memory,
            BlockStage::A(_) => Self::FileSystem,
        }
    }
}

pub async fn app(config: &Figment) -> Result<Rocket<Build>> {
    let kepler_config: Config = config.extract::<Config>()?;

    tracing::tracing_try_init(&kepler_config.log);

    let kp = kepler_config.storage.blocks.relay_key_pair().await?;

    let relay_node = RelayNode::new(kepler_config.relay.port, Keypair::Ed25519(kp)).await?;
    let db = Database::connect(&kepler_config.storage.database).await?;
    Migrator::up(&db, None).await?;

    let routes = routes![
        healthcheck,
        cors,
        relay_addr,
        open_host_key,
        invoke,
        delegate,
    ];

    let rocket = rocket::custom(config)
        .mount("/", routes)
        .attach(AdHoc::config::<config::Config>())
        .attach(tracing::TracingFairing {
            header_name: kepler_config.log.tracing.traceheader,
        })
        .manage(db)
        .manage(relay_node)
        .manage(RwLock::new(HashMap::<PeerId, Ed25519Keypair>::new()));

    if kepler_config.cors {
        Ok(rocket.attach(AdHoc::on_response("CORS", |_, resp| {
            Box::pin(async move {
                resp.set_header(Header::new("Access-Control-Allow-Origin", "*"));
                resp.set_header(Header::new(
                    // allow these methods for requests
                    "Access-Control-Allow-Methods",
                    "POST, PUT, GET, OPTIONS, DELETE",
                ));
                resp.set_header(Header::new(
                    // expose response headers to browser-run scripts
                    "Access-Control-Expose-Headers",
                    "*, Authorization",
                ));
                resp.set_header(Header::new(
                    // allow custom headers + Authorization in requests
                    "Access-Control-Allow-Headers",
                    "*, Authorization",
                ));
                resp.set_header(Header::new("Access-Control-Allow-Credentials", "true"));
            })
        })))
    } else {
        Ok(rocket)
    }
}
