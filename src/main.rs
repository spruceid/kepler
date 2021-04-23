#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate tokio;

use anyhow::{anyhow, Error, Result};
use libipld::{
    cid::{multibase::Base, Cid},
    store::StoreParams,
};
use rocket::{
    data::{ByteUnit, Data, ToByteUnit},
    fairing::Fairing,
    form::Form,
    futures::stream::StreamExt,
    launch,
    response::{Debug, Stream},
    request::{FromRequest, Outcome, Request},
    State,
    http::Status,
};
use rocket_cors::CorsOptions;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    str::FromStr,
};

mod auth;
mod cas;
mod codec;
mod ipfs;
mod tz;
mod orbit;

use auth::AuthToken;
use cas::ContentAddressedStorage;
use codec::{PutContent, SupportedCodecs};
use ipfs_embed::{Config, DefaultParams, Ipfs, Multiaddr, PeerId};
use orbit::{Orbit, SimpleOrbit};

struct CidWrap(Cid);

// Orphan rule requires a wrapper type for this :(
impl<'a> rocket::request::FromParam<'a> for CidWrap {
    type Error = anyhow::Error;
    fn from_param(param: &'a str) -> Result<CidWrap> {
        Ok(CidWrap(Cid::from_str(param)?))
    }
}

struct Orbits<'a, O>
where
    O: Orbit,
{
    stores: BTreeMap<&'a Cid, O>,
    base_path: PathBuf,
}

impl<'a, O: Orbit> Orbits<'a, O> {
    fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            stores: BTreeMap::new(),
            base_path: path.as_ref().to_path_buf(),
        }
    }

    fn orbit(&self, id: &Cid) -> Option<&O> {
        self.stores.get(id)
    }

    fn add(&mut self, id: &'a Cid, orbit: O) {
        self.stores.insert(id, orbit);
    }
}

#[get("/<orbit_id>/<hash>")]
async fn get_content(
    orbit_id: CidWrap,
    hash: CidWrap,
    auth: AuthToken,
    orbit: &SimpleOrbit
) -> Result<Option<Stream<Cursor<Vec<u8>>>>, Debug<Error>> {
        match ContentAddressedStorage::get(orbit, &hash.0).await {
            Ok(Some(content)) => Ok(Some(Stream::chunked(Cursor::new(content.to_owned()), 1024))),
            Ok(None) => Ok(None),
            Err(e) => Err(e)?,
        }
}

#[post("/<orbit_id>", format = "multipart/form-data", data = "<batch>")]
async fn batch_put_content(
    orbit_id: CidWrap,
    batch: Form<Vec<PutContent>>,
    auth: AuthToken,
    orbit: &SimpleOrbit
) -> Result<String, Debug<Error>> {
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
    orbit_id: CidWrap,
    data: Data,
    codec: SupportedCodecs,
    auth: AuthToken,
    orbit: &SimpleOrbit
) -> Result<String, Debug<Error>> {
    match orbit.put(&mut data.open(10u8.megabytes()), codec).await {
        Ok(cid) => Ok(cid
            .to_string_of_base(Base::Base64Url)
            .map_err(|e| anyhow!(e))?),
        Err(e) => Err(e)?,
    }
}

#[delete("/<orbit_id>/<hash>")]
async fn delete_content(
    orbit_id: CidWrap,
    hash: CidWrap,
    auth: AuthToken,
    orbit: &SimpleOrbit
) -> Result<(), Debug<Error>> {
    Ok(orbit.delete(&hash.0).await?)
}

#[async_std::main]
async fn main() -> Result<()> {
    let rocket_config = rocket::Config::figment();

    #[derive(Deserialize, Debug)]
    struct DBConfig {
        db_path: std::path::PathBuf,
    }

    let path = rocket_config
        .extract::<DBConfig>()
        .expect("db path missing").db_path;

    rocket::tokio::runtime::Runtime::new()?
        .spawn(
            rocket::custom(rocket_config)
                .manage(Orbits::<'_, SimpleOrbit>::new(&path))
                .mount(
                    "/",
                    routes![get_content, put_content, batch_put_content, delete_content],
                )
                .attach(CorsOptions::default().to_cors()?)
                .launch(),
        )
        .await??;
    Ok(())
}
