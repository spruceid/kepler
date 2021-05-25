#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate rocket;

use anyhow::Result;
use rocket::{fairing::AdHoc, figment::Figment, http::Header, Build, Rocket};

pub mod auth;
pub mod cas;
pub mod codec;
pub mod config;
pub mod ipfs;
pub mod lib;
pub mod orbit;
pub mod routes;
pub mod tz;

use crate::config::Config;
use crate::orbit::load_orbits;
use crate::routes::{
    batch_put_content, batch_put_create, cors, delete_content, get_content, put_content, put_create,
};
use crate::tz::TezosBasicAuthorization;

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
