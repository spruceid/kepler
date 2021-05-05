#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate tokio;

use anyhow::{anyhow, Error, Result};
use libipld::cid::{
    multibase::Base,
    multihash::{Code, MultihashDigest},
    Cid,
};
use rocket::{
    data::{Data, ToByteUnit},
    form::Form,
    futures::stream::StreamExt,
    response::{Debug, Stream as RocketStream},
    tokio::fs::read_dir,
    State,
};
use rocket_cors::CorsOptions;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io::Cursor,
    path::{Path, PathBuf},
    str::FromStr,
};
use tokio::sync::{RwLock, RwLockReadGuard};
use tokio_stream::wrappers::ReadDirStream;
use tz::{TZAuth, TezosBasicAuthorization};

mod auth;
mod cas;
mod codec;
mod ipfs;
mod orbit;
mod tz;

use auth::{Action, AuthWrapper, AuthorizationToken};
use cas::ContentAddressedStorage;
use codec::{PutContent, SupportedCodecs};
use orbit::{create_orbit, Orbit, SimpleOrbit};

struct CidWrap(Cid);

// Orphan rule requires a wrapper type for this :(
impl<'a> rocket::request::FromParam<'a> for CidWrap {
    type Error = anyhow::Error;
    fn from_param(param: &'a str) -> Result<CidWrap> {
        Ok(CidWrap(Cid::from_str(param)?))
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
            // filter for those with valid CID filenames
            .filter_map(|p| async { Cid::from_str(p.ok()?.file_name().to_str()?).ok() })
            // get a future to load each
            .filter_map(|cid| async move {
                create_orbit(cid, path_ref, TezosBasicAuthorization)
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
    _auth: Option<AuthWrapper<TZAuth>>,
) -> Result<Option<RocketStream<Cursor<Vec<u8>>>>, Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or(anyhow!("No Orbit Found"))?;
    match ContentAddressedStorage::get(orbit, &hash.0).await {
        Ok(Some(content)) => Ok(Some(RocketStream::chunked(
            Cursor::new(content.to_owned()),
            1024,
        ))),
        Ok(None) => Ok(None),
        Err(e) => Err(e)?,
    }
}

#[post("/<orbit_id>", format = "multipart/form-data", data = "<batch>")]
async fn batch_put_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    batch: Form<Vec<PutContent>>,
    _auth: AuthWrapper<TZAuth>,
) -> Result<String, Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or(anyhow!("No Orbit Found"))?;
    let mut cids = Vec::<String>::new();
    for mut content in batch.into_inner().into_iter() {
        cids.push(
            orbit
                .put(&mut content.content, content.codec)
                .await
                .map_or("".into(), |cid| {
                    cid.to_string_of_base(Base::Base64Url)
                        .map_or("".into(), |s| s)
                }),
        );
    }
    Ok(cids.join("\n"))
}

#[post("/<orbit_id>", data = "<data>", rank = 2)]
async fn put_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    data: Data,
    codec: SupportedCodecs,
    _auth: AuthWrapper<TZAuth>,
) -> Result<String, Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or(anyhow!("No Orbit Found"))?;
    match orbit.put(&mut data.open(10u8.megabytes()), codec).await {
        Ok(cid) => Ok(cid
            .to_string_of_base(Base::Base64Url)
            .map_err(|e| anyhow!(e))?),
        Err(e) => Err(e)?,
    }
}

#[post("/", format = "multipart/form-data", data = "<batch>")]
async fn batch_put_create(
    // TODO find a good way to not restrict all orbits to the same Type
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    batch: Form<Vec<PutContent>>,
    auth: AuthWrapper<TZAuth>,
) -> Result<String, Debug<Error>> {
    match auth.0.action() {
        Action::Create { pkh, salt, put } => {
            let orbit = create_orbit(
                Cid::new_v1(
                    SupportedCodecs::Raw as u64,
                    Code::Blake3_256.digest([&pkh, ":", &salt].join("").as_bytes()),
                ),
                &orbits.base_path,
                TezosBasicAuthorization,
            )
            .await?;

            let mut cids = Vec::<String>::new();
            for mut content in batch.into_inner().into_iter() {
                cids.push(orbit.put(&mut content.content, content.codec).await.map_or(
                    "".into(),
                    |cid| {
                        cid.to_string_of_base(Base::Base64Url)
                            .map_or("".into(), |s| s)
                    },
                ));
            }
            orbits.add(orbit);
            Ok(cids.join("\n"))
        }
        _ => Err(anyhow!("Invalid Authorization"))?,
    }
}

#[post("/", data = "<data>", rank = 2)]
async fn put_create(
    // TODO find a good way to not restrict all orbits to the same Type
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    data: Data,
    codec: SupportedCodecs,
    auth: AuthWrapper<TZAuth>,
) -> Result<String, Debug<Error>> {
    match auth.0.action() {
        Action::Create { pkh, salt, put } => {
            let orbit = create_orbit(
                Cid::new_v1(
                    SupportedCodecs::Raw as u64,
                    Code::Blake3_256.digest([&pkh, ":", &salt].join("").as_bytes()),
                ),
                &orbits.base_path,
                TezosBasicAuthorization,
            )
            .await?;

            let cid = orbit.put(&mut data.open(10u8.megabytes()), codec).await?;

            orbits.add(orbit);

            Ok(cid
                .to_string_of_base(Base::Base64Url)
                .map_err(|e| anyhow!(e))?)
        }
        _ => Err(anyhow!("Invalid Authorization"))?,
    }
}

#[delete("/<orbit_id>/<hash>")]
async fn delete_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    auth: AuthWrapper<TZAuth>,
) -> Result<(), Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or(anyhow!("No Orbit Found"))?;
    Ok(orbit.delete(&hash.0).await?)
}

#[rocket::main]
async fn main() -> Result<()> {
    let rocket_config = rocket::Config::figment();

    #[derive(Deserialize, Debug)]
    struct DBConfig {
        db_path: std::path::PathBuf,
    }

    let path = rocket_config
        .extract::<DBConfig>()
        .expect("db path missing")
        .db_path;

    rocket::custom(rocket_config)
        .manage(load_orbits(path).await?)
        .manage(TezosBasicAuthorization)
        .mount(
            "/",
            routes![get_content, put_content, batch_put_content, delete_content],
        )
        .attach(CorsOptions::default().to_cors()?)
        .launch()
        .await?;

    Ok(())
}
