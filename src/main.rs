#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate tokio;

use anyhow::{anyhow, Error, Result};
use libipld::cid::{multibase::Base, Cid};
use rocket::{
    data::{ByteUnit, Data, ToByteUnit},
    fairing::Fairing,
    form::Form,
    launch,
    response::{Debug, Stream},
    State,
};
use rocket_cors::CorsOptions;
use std::{
    io::{Cursor, Read},
    str::FromStr,
};

mod auth;
mod cas;
mod codec;
mod ipfs;
mod tz;

use auth::AuthToken;
use cas::ContentAddressedStorage;
use codec::{PutContent, SupportedCodecs};
use ipfs_embed::{Config, DefaultParams, Ipfs, Multiaddr, PeerId};

const DB_PATH: &'static str = "/tmp/kepler_cas";

struct CidWrap(Cid);

// Orphan rule requires a wrapper type for this :(
impl<'a> rocket::request::FromParam<'a> for CidWrap {
    type Error = anyhow::Error;
    fn from_param(param: &'a str) -> Result<CidWrap> {
        Ok(CidWrap(Cid::from_str(param)?))
    }
}

struct Store<T>
where
    T: ContentAddressedStorage,
{
    pub db: T,
}

#[get("/<orbit_id>/<hash>")]
async fn get_content(
    state: State<'_, Store<Ipfs<DefaultParams>>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    auth: AuthToken,
) -> Result<Option<Stream<Cursor<Vec<u8>>>>, Debug<Error>> {
    match ContentAddressedStorage::get(&state.db, &hash.0).await {
        Ok(Some(content)) => Ok(Some(Stream::chunked(Cursor::new(content.to_owned()), 1024))),
        Ok(None) => Ok(None),
        Err(e) => Err(e)?,
    }
}

#[post("/<orbit_id>", format = "multipart/form-data", data = "<batch>")]
async fn batch_put_content(
    state: State<'_, Store<Ipfs<DefaultParams>>>,
    orbit_id: CidWrap,
    batch: Form<Vec<PutContent>>,
    auth: AuthToken,
) -> Result<String, Debug<Error>> {
    let mut cids = Vec::<String>::new();
    for mut content in batch.into_inner().into_iter() {
        cids.push(
            state
                .db
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
    state: State<'_, Store<Ipfs<DefaultParams>>>,
    orbit_id: CidWrap,
    data: Data,
    codec: SupportedCodecs,
    auth: AuthToken,
) -> Result<String, Debug<Error>> {
    match state.db.put(&mut data.open(10u8.megabytes()), codec).await {
        Ok(cid) => Ok(cid
            .to_string_of_base(Base::Base64Url)
            .map_err(|e| anyhow!(e))?),
        Err(e) => Err(e)?,
    }
}

#[delete("/<orbit_id>/<hash>")]
async fn delete_content(
    state: State<'_, Store<Ipfs<DefaultParams>>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    auth: AuthToken,
) -> Result<(), Debug<Error>> {
    Ok(state.db.delete(&hash.0).await?)
}

#[async_std::main]
async fn main() -> Result<()> {
    let mut cfg = Config::new(None, 10);
    // cfg.network.enable_kad = false;
    let ipfs = Ipfs::<DefaultParams>::new(cfg).await?;
    let peer: PeerId = "QmRSGx67Kq8w7xSBDia7hQfbfuvauMQGgxcwSWw976x4BS".parse()?;
    let addr: Multiaddr = "/ip4/54.173.33.96/tcp/4001".parse()?;
    ipfs.bootstrap(&[(peer, addr)]).await?;

    let cors = CorsOptions::default().to_cors()?;

    rocket::tokio::runtime::Runtime::new()?
        .spawn(
            rocket::ignite()
                .manage(Store { db: ipfs })
                .mount(
                    "/",
                    routes![get_content, put_content, batch_put_content, delete_content],
                )
                .attach(cors)
                .launch(),
        )
        .await??;
    Ok(())
}
