#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
extern crate cid;
extern crate did_pkh;
extern crate multihash;
extern crate rocksdb;
extern crate ssi;
#[macro_use]
extern crate tokio;
extern crate bs58;
extern crate nom;
extern crate serde_json;
#[macro_use]
extern crate hex;

use anyhow::Result;
use cid::multibase::Base;
use cid::Cid;
use rocket::{data::Data, http::RawStr, response::Stream, State};
use std::{
    io::{Cursor, Read},
    str::FromStr,
};

mod auth;
mod cas;
mod codec;
mod tz;

use auth::AuthToken;
use cas::{ContentAddressedStorage, CASDB};
use codec::SupportedCodecs;

// 10 megabytes
const STREAM_LIMIT: u64 = 10000000;
const DB_PATH: &'static str = "/tmp/kepler_cas";

struct CidWrap(Cid);

// Orphan rule requires a wrapper type for this :(
impl<'a> rocket::request::FromParam<'a> for CidWrap {
    type Error = anyhow::Error;
    fn from_param(param: &'a RawStr) -> Result<CidWrap> {
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
fn get_content(
    state: State<Store<CASDB>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    auth: AuthToken,
) -> Result<Option<Stream<Cursor<Vec<u8>>>>> {
    match state.db.get(hash.0) {
        Ok(Some(content)) => Ok(Some(Stream::chunked(Cursor::new(content.to_owned()), 1024))),
        Ok(None) => Ok(None),
        Err(e) => Err(e),
    }
}

#[post("/<orbit_id>", data = "<data>")]
fn put_content(
    state: State<Store<CASDB>>,
    orbit_id: CidWrap,
    data: Data,
    codec: SupportedCodecs,
    auth: AuthToken,
) -> Result<String> {
    match state.db.put(data.open().take(STREAM_LIMIT), codec) {
        Ok(cid) => Ok(cid.to_string_of_base(Base::Base64Url)?),
        Err(e) => Err(e),
    }
}

#[delete("/<orbit_id>/<hash>")]
fn delete_content(
    state: State<Store<CASDB>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    auth: AuthToken,
) -> Result<()> {
    Ok(state.db.delete(hash.0)?)
}

fn main() {
    rocket::ignite()
        .manage(Store {
            db: CASDB::new(DB_PATH).unwrap(),
        })
        .mount("/", routes![get_content, put_content, delete_content])
        .launch();
}
