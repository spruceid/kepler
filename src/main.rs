#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
extern crate cid;
extern crate multihash;
extern crate rocksdb;

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

use auth::{Authorization, DummyAuth};
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

#[get("/<hash>")]
fn get_content(
    state: State<Store<CASDB>>,
    hash: CidWrap,
    auth: Authorization<DummyAuth>,
) -> Result<Option<Stream<Cursor<Vec<u8>>>>> {
    match state.db.get(hash.0) {
        Ok(Some(content)) => Ok(Some(Stream::chunked(Cursor::new(content.to_owned()), 1024))),
        Ok(None) => Ok(None),
        Err(e) => Err(e),
    }
}

#[post("/", data = "<data>")]
fn put_content(
    state: State<Store<CASDB>>,
    data: Data,
    codec: SupportedCodecs,
    auth: Authorization<DummyAuth>,
) -> Result<String> {
    match state.db.put(data.open().take(STREAM_LIMIT), codec) {
        Ok(cid) => Ok(cid.to_string_of_base(Base::Base64Url)?),
        Err(e) => Err(e),
    }
}

fn main() {
    rocket::ignite()
        .manage(Store {
            db: CASDB::new(DB_PATH).unwrap(),
        })
        .mount("/", routes![get_content])
        .mount("/", routes![put_content])
        .launch();
}
