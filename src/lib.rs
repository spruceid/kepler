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
pub mod cas;
pub mod codec;
pub mod config;
pub mod ipfs;
pub mod orbit;
pub mod relay;
pub mod routes;
pub mod s3;
pub mod tz;
pub mod tz_orbit;
pub mod zcap;

use ipfs_embed::{generate_keypair, Keypair, PeerId, ToLibp2p};
use relay::RelayNode;
use routes::core::{
    batch_put_content, cors, delete_content, get_content, get_content_no_auth, list_content,
    list_content_no_auth, open_host_key, open_orbit_allowlist, open_orbit_authz, put_content,
    relay_addr,
};
use routes::s3 as s3_routes;
use std::{collections::HashMap, sync::RwLock};

pub fn tracing_try_init() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();
}

pub async fn app(config: &Figment) -> Result<Rocket<Build>> {
    let kepler_config = config.extract::<config::Config>()?;

    // ensure KEPLER_DATABASE_PATH exists
    if !kepler_config.database.path.is_dir() {
        return Err(anyhow!(
            "KEPLER_DATABASE_PATH does not exist or is not a directory: {}",
            kepler_config.database.path.to_str().unwrap()
        ));
    }

    let kp: Keypair = if let Ok(bytes) = fs::read(kepler_config.database.path.join("kp")).await {
        Keypair::from_bytes(&bytes)?
    } else {
        let kp = generate_keypair();
        fs::write(kepler_config.database.path.join("kp"), kp.to_bytes()).await?;
        kp
    };

    let relay_node = RelayNode::new(kepler_config.relay.port, kp.to_keypair())?;

    let mut routes = routes![
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
        .manage(RwLock::new(HashMap::<PeerId, Keypair>::new())))
}

#[test]
#[should_panic]
async fn test_form() {
    use codec::PutContent;
    use rocket::{form::Form, http::ContentType, local::asynchronous::Client};

    #[post("/", format = "multipart/form-data", data = "<form>")]
    async fn stub_batch(form: Form<Vec<PutContent>>) {
        let content1 = &form.get(0).unwrap().content.value;
        let content2 = &form.get(1).unwrap().content.value;
        let p1 = r#"{"dummy":"obj"}"#;
        let p2 = r#"{"amother":"obj"}"#;
        assert_eq!(&content1, &p1.as_bytes());
        assert_eq!(&content2, &p2.as_bytes());
    }

    let form = r#"
-----------------------------28081028282221432566755324225
Content-Disposition: form-data; name="zyop8PQypg8QWqGNG92jJacYtEa56Mnaf9tLxDadXc8kPPxNVWZye"; filename="blob"
Content-Type: application/json

{"dummy":"obj"}
-----------------------------28081028282221432566755324225
Content-Disposition: form-data; name="zyop8PQypZnwFc58SPAxZTSCuG6R13jWSxQp8iBGNmBuV3HsrVyLx"; filename="blob"
Content-Type: application/json

{"amother":"obj"}
-----------------------------28081028282221432566755324225--
"#;

    let client = Client::debug_with(rocket::routes![stub_batch])
        .await
        .unwrap();
    let res = client
        .post("/")
        .header(
            "multipart/form-data; boundary=-----------------------------28081028282221432566755324225"
                .parse::<ContentType>()
                .unwrap()
        )
        .body(&form)
        .dispatch()
        .await;

    assert!(res.status().class().is_success());
}
