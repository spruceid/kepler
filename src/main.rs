#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate tokio;

use anyhow::{anyhow, Error, Result};
use libipld::cid::{Cid, multibase::Base};
use rocket::{
    data::{ByteUnit, Data, ToByteUnit},
    http::{ContentType, RawStr},
    launch,
    response::{Debug, Stream},
    State,
};
use std::{
    io::{Cursor, Read},
    str::FromStr,
};

mod auth;
mod cas;
mod codec;
mod tz;

use auth::AuthToken;
use cas::ContentAddressedStorage;
use codec::SupportedCodecs;
use ipfs_embed::{Config, DefaultParams, Ipfs};

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
    match ContentAddressedStorage::get(&state.db, hash.0).await {
        Ok(Some(content)) => Ok(Some(Stream::chunked(Cursor::new(content.to_owned()), 1024))),
        Ok(None) => Ok(None),
        Err(e) => Err(e)?,
    }
}

#[put("/<orbit_id>", data = "<data>")]
async fn put_content(
    state: State<'_, Store<Ipfs<DefaultParams>>>,
    orbit_id: CidWrap,
    data: Data,
    codec: SupportedCodecs,
    auth: AuthToken,
) -> Result<String, Debug<Error>> {
    match state.db.put(data.open(10.megabytes()), codec).await {
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
    Ok(state.db.delete(hash.0).await?)
}

#[async_std::main]
async fn main() -> Result<()> {
    let ipfs = Ipfs::<DefaultParams>::new(Config::new(None, 10)).await?;
    ipfs.listen_on("/ip4/0.0.0.0/tcp/0".parse()?).await?;

    rocket::tokio::runtime::Runtime::new()?
        .spawn(
            rocket::ignite()
                .manage(Store { db: ipfs })
                .mount("/", routes![get_content, put_content, delete_content])
                .launch(),
        )
        .await??;
    Ok(())
}
