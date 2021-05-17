#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate tokio;

use anyhow::{anyhow, Error, Result};
use libipld::cid::Cid;
use rocket::{
    data::{Data, ToByteUnit},
    fairing::AdHoc,
    figment::providers::{Env, Format, Serialized, Toml},
    form::{DataField, Form, FromFormField},
    futures::stream::StreamExt,
    http::Header,
    response::{Debug, Stream as RocketStream},
    tokio::fs::read_dir,
    State,
};
use ssi::did::DIDURL;
// use rocket_cors::CorsOptions;
use std::{
    collections::BTreeMap,
    io::Cursor,
    path::{Path, PathBuf},
    str::FromStr,
};
use tokio::sync::{RwLock, RwLockReadGuard};
use tokio_stream::wrappers::ReadDirStream;
use tz::{TezosAuthorizationString, TezosBasicAuthorization};

mod auth;
mod cas;
mod codec;
mod config;
mod ipfs;
mod orbit;
mod tz;

use auth::{Action, AuthWrapper, AuthorizationToken};
use cas::ContentAddressedStorage;
use codec::{PutContent, SupportedCodecs};
use orbit::{create_orbit, load_orbit, verify_oid_v0, Orbit, SimpleOrbit};

#[derive(PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CidWrap(Cid);

// Orphan rule requires a wrapper type for this :(
impl<'a> rocket::request::FromParam<'a> for CidWrap {
    type Error = anyhow::Error;
    fn from_param(param: &'a str) -> Result<CidWrap> {
        Ok(CidWrap(Cid::from_str(param)?))
    }
}

#[rocket::async_trait]
impl<'r> FromFormField<'r> for CidWrap {
    async fn from_data(field: DataField<'r, '_>) -> rocket::form::Result<'r, Self> {
        Ok(CidWrap(
            field
                .name
                .source()
                .parse()
                .map_err(|_| field.unexpected())?,
        ))
    }
}

struct Orbits<O>
where
    O: Orbit,
{
    pub stores: RwLock<BTreeMap<Cid, O>>,
    pub base_path: PathBuf,
}

#[rocket::async_trait]
pub trait OrbitCollection<O: Orbit> {
    async fn orbits(&self) -> RwLockReadGuard<BTreeMap<Cid, O>>;
    async fn add(&self, orbit: O) -> ();
}

impl<O: Orbit> Orbits<O> {
    fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            stores: RwLock::new(BTreeMap::new()),
            base_path: path.as_ref().to_path_buf(),
        }
    }
}

#[rocket::async_trait]
impl<O: Orbit> OrbitCollection<O> for Orbits<O> {
    async fn orbits(&self) -> RwLockReadGuard<BTreeMap<Cid, O>> {
        self.stores.read().await
    }

    // fn orbit(&self, id: &Cid) -> Option<&O> {
    //     self.stores.read().expect("read orbit set").get(id)
    // }

    async fn add(&self, orbit: O) {
        let mut lock = self.stores.write().await;
        lock.insert(*orbit.id(), orbit);
    }
}

async fn load_orbits<P: AsRef<Path>>(
    path: P,
) -> Result<Orbits<SimpleOrbit<TezosBasicAuthorization>>> {
    let path_ref: &Path = path.as_ref();
    let orbits = Orbits::new(path_ref);
    // for entries in the dir
    let orbit_list: Vec<SimpleOrbit<TezosBasicAuthorization>> =
        ReadDirStream::new(read_dir(path_ref).await?)
            // try to load each as an orbit
            .filter_map(|p| async {
                load_orbit(p.ok()?.file_name().to_str()?, TezosBasicAuthorization)
                    .await
                    .ok()
            })
            // load them all
            .collect()
            .await;

    for orbit in orbit_list.into_iter() {
        orbits.add(orbit).await
    }

    Ok(orbits)
}

#[get("/<orbit_id>/<hash>")]
async fn get_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    _auth: Option<AuthWrapper<TezosAuthorizationString>>,
) -> Result<Option<RocketStream<Cursor<Vec<u8>>>>, Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or_else(|| anyhow!("No Orbit Found"))?;
    match orbit.get(&hash.0).await {
        Ok(Some(content)) => Ok(Some(RocketStream::chunked(Cursor::new(content), 1024))),
        Ok(None) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[post("/<orbit_id>", format = "multipart/form-data", data = "<batch>")]
async fn batch_put_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    batch: Form<Vec<PutContent>>,
    _auth: AuthWrapper<TezosAuthorizationString>,
) -> Result<String, Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or_else(|| anyhow!("No Orbit Found"))?;
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
) -> Result<String, Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or_else(|| anyhow!("No Orbit Found"))?;
    match orbit
        .put(
            &data
                .open(10u8.megabytes())
                .into_bytes()
                .await
                .map_err(|e| anyhow!(e))?,
            codec,
        )
        .await
    {
        Ok(cid) => Ok(orbit.make_uri(&cid)?),
        Err(e) => Err(e.into()),
    }
}

#[post("/", format = "multipart/form-data", data = "<batch>")]
async fn batch_put_create(
    // TODO find a good way to not restrict all orbits to the same Type
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    batch: Form<Vec<PutContent>>,
    auth: AuthWrapper<TezosAuthorizationString>,
) -> Result<String, Debug<Error>> {
    match auth.0.action() {
        Action::Create {
            orbit_id,
            parameters,
            ..
        } => {
            verify_oid_v0(orbit_id, &auth.0.pkh, parameters)?;

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
            .await?;

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
        _ => Err(anyhow!("Invalid Authorization").into()),
    }
}

#[post("/", data = "<data>", rank = 2)]
async fn put_create(
    // TODO find a good way to not restrict all orbits to the same Type
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    data: Data,
    codec: SupportedCodecs,
    auth: AuthWrapper<TezosAuthorizationString>,
) -> Result<String, Debug<Error>> {
    match auth.0.action() {
        Action::Create {
            orbit_id,
            parameters,
            ..
        } => {
            verify_oid_v0(orbit_id, &auth.0.pkh, parameters)?;

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
            .await?;

            let uri = orbit.make_uri(
                &orbit
                    .put(
                        &data
                            .open(10u8.megabytes())
                            .into_bytes()
                            .await
                            .map_err(|e| anyhow!(e))?,
                        codec,
                    )
                    .await?,
            )?;

            orbits.add(orbit).await;

            Ok(uri)
        }
        _ => Err(anyhow!("Invalid Authorization").into()),
    }
}

#[delete("/<orbit_id>/<hash>")]
async fn delete_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    _auth: AuthWrapper<TezosAuthorizationString>,
) -> Result<(), Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or_else(|| anyhow!("No Orbit Found"))?;
    Ok(orbit.delete(&hash.0).await?)
}

#[options("/<_s..>")]
async fn cors(_s: PathBuf) -> Result<(), Debug<Error>> {
    Ok(())
}

#[rocket::main]
async fn main() {
    let config = rocket::figment::Figment::from(rocket::Config::default())
        .merge(Serialized::defaults(config::Config::default()))
        .merge(Toml::file("kepler.toml").nested())
        .merge(Env::prefixed("KEPLER_").split("_").global())
        .merge(Env::prefixed("ROCKET_").global()); // That's just for easy access to ROCKET_LOG_LEVEL

    let kepler_config = config.extract::<config::Config>().unwrap();

    // ensure KEPLER_DATABASE_PATH exists
    if !kepler_config.database.path.is_dir() {
        panic!(
            "KEPLER_DATABASE_PATH does not exist or is not a directory: {}",
            kepler_config.database.path.to_str().unwrap()
        );
    }

    rocket::custom(config.clone())
        .manage(load_orbits(kepler_config.database.path).await.unwrap())
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
        }))
        .launch()
        .await
        .unwrap();
}

#[test]
#[should_panic]
async fn test_form() {
    use rocket::{http::ContentType, local::asynchronous::Client};

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
