#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
extern crate multibase;
extern crate multihash;
extern crate rocksdb;

use anyhow::Result;
use multihash::{Blake3Hasher, Code, Multihash, MultihashDigest, StatefulHasher};
use rocket::{data::Data, http::RawStr, response::Stream, State};
use rocksdb::{Options, DB};
use std::sync::{Arc, Mutex};
use std::{convert::TryFrom, fmt::Display, io::Read};

// 10 megabytes
const STREAM_LIMIT: u64 = 10000000;
const DB_PATH: &'static str = "/tmp/kepler_cas";

struct MH(Multihash);

impl MH {
    pub fn decode(r: &[u8]) -> Result<Self, multihash::Error> {
        Ok(Self(Multihash::from_bytes(r)?))
    }
}

// Orphan rule requires a wrapper type for this :(
impl<'a> rocket::request::FromParam<'a> for MH {
    type Error = anyhow::Error;
    fn from_param(param: &'a RawStr) -> Result<MH> {
        MH::decode(&multibase::decode(param)?.1).map_err(|e| e.into())
    }
}

struct Store {
    pub db: Arc<Mutex<DB>>,
}

#[get("/<hash>")]
fn get_content(hash: MH) -> Option<NamedFile> {
    todo!()
}

#[post("/", format = "plain", data = "<data>")]
fn put_content(data: Data) -> String {
    todo!()
}

fn main() {
    rocket::ignite()
        .manage(Store {
            db: Arc::new(Mutex::new(DB::open_default(DB_PATH).unwrap())),
        })
        .mount("/", routes![get_content])
        .mount("/", routes![put_content])
        .launch();
}
