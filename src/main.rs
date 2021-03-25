#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
extern crate multibase;
extern crate multihash;
extern crate rocksdb;

use anyhow::Result;
use multihash::{Blake3_256, Code, Multihash, MultihashDigest, StatefulHasher};
use rocket::{data::Data, http::RawStr, response::Stream, State};
use rocksdb::{Options, DB};
use std::sync::{Arc, Mutex, MutexGuard};
use std::{
    convert::TryFrom,
    fmt::Display,
    io::{Cursor, Read},
    ops::Deref,
};

mod cas;

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

#[derive(Clone)]
struct Store {
    pub db: Arc<Mutex<DB>>,
}

impl cas::ContentAddressedStorage for &Store {
    type Error = anyhow::Error;
    fn put<C: Read>(&self, content: C) -> Result<Multihash, Self::Error> {
        let c: Vec<u8> = content.bytes().filter_map(|b| b.ok()).collect();
        let hash = Code::Blake3_256.digest(&c);

        self.db
            .lock()
            .map_err(|e| anyhow!(format!("{}", e)))?
            .put(&hash.to_bytes(), &c)?;

        Ok(hash)
    }
    fn get(&self, digest: Multihash) -> Result<Option<Vec<u8>>, Self::Error> {
        match self
            .db
            .lock()
            .map_err(|e| anyhow!(format!("{}", e)))?
            .get(digest.to_bytes())
        {
            Ok(Some(content)) => {
                if Code::try_from(digest.code())?.digest(&content) != digest {
                    Err(anyhow!("Invalid Content Address"))
                } else {
                    Ok(Some(content.to_vec()))
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn delete(&self, digest: Multihash) -> Result<(), Self::Error> {
        self.db
            .lock()
            .map_err(|e| anyhow!(format!("{}", e)))?
            .delete(digest.to_bytes())
            .map_err(|e| e.into())
    }
}

#[get("/<hash>")]
fn get_content(state: State<Store>, hash: MH) -> Result<Option<Stream<Cursor<Vec<u8>>>>> {
    match cas::ContentAddressedStorage::get(&state.deref(), hash.0) {
        Ok(Some(content)) => Ok(Some(Stream::chunked(Cursor::new(content), 1024))),
        Ok(None) => Ok(None),
        Err(e) => Err(e),
    }
}

#[post("/", format = "binary", data = "<data>")]
fn put_content(state: State<Store>, data: Data) -> Result<String> {
    match cas::ContentAddressedStorage::put(&state.deref(), data.open().take(STREAM_LIMIT)) {
        Ok(hash) => Ok(multibase::encode(multibase::Base::Base64, hash.to_bytes())),
        Err(e) => Err(e),
    }
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
