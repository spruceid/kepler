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
use cid::{Cid, Version};
use multihash::{Code, Multihash, MultihashDigest};
use rocket::{
    data::Data,
    http::{ContentType, RawStr, Status},
    request::{FromRequest, Outcome, Request},
    response::Stream,
    State,
};
use rocksdb::{Options, DB};
use std::{
    convert::TryFrom,
    io::{Cursor, Read},
    ops::Deref,
    str::FromStr,
    sync::{Arc, Mutex},
};

mod cas;

// 10 megabytes
const STREAM_LIMIT: u64 = 10000000;
const DB_PATH: &'static str = "/tmp/kepler_cas";

struct MH(Cid);

// Orphan rule requires a wrapper type for this :(
impl<'a> rocket::request::FromParam<'a> for MH {
    type Error = anyhow::Error;
    fn from_param(param: &'a RawStr) -> Result<MH> {
        Ok(MH(Cid::from_str(param)?))
    }
}

#[derive(Clone)]
struct Store {
    pub db: Arc<Mutex<DB>>,
}

impl cas::ContentAddressedStorage for &Store {
    type Error = anyhow::Error;
    fn put<C: Read>(&self, content: C, codec: SupportedCodecs) -> Result<Cid, Self::Error> {
        let c: Vec<u8> = content.bytes().filter_map(|b| b.ok()).collect();
        let cid = Cid::new(Version::V1, codec as u64, Code::Blake3_256.digest(&c))?;

        self.db
            .lock()
            .map_err(|e| anyhow!(format!("{}", e)))?
            .put(&cid.to_bytes(), &c)?;

        Ok(cid)
    }
    fn get(&self, address: Cid) -> Result<Option<Vec<u8>>, Self::Error> {
        match self
            .db
            .lock()
            .map_err(|e| anyhow!(format!("{}", e)))?
            .get(address.to_bytes())
        {
            Ok(Some(content)) => {
                if Code::try_from(address.hash().code())?.digest(&content) != *address.hash() {
                    Err(anyhow!("Invalid Content Address"))
                } else {
                    Ok(Some(content.to_vec()))
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn delete(&self, address: Cid) -> Result<(), Self::Error> {
        self.db
            .lock()
            .map_err(|e| anyhow!(format!("{}", e)))?
            .delete(address.to_bytes())
            .map_err(|e| e.into())
    }
}

enum SupportedCodecs {
    Raw = 0x55,
    Json = 0x0200,
    MsgPack = 0x0201,
    Cbor = 0x51,
}

impl From<&ContentType> for SupportedCodecs {
    fn from(c: &ContentType) -> Self {
        if c.is_json() {
            Self::Json
        } else if c.is_msgpack() {
            Self::MsgPack
        } else {
            Self::Raw
        }
    }
}

impl<'a, 'r> FromRequest<'a, 'r> for SupportedCodecs {
    type Error = anyhow::Error;

    fn from_request(req: &'a Request<'r>) -> Outcome<Self, Self::Error> {
        Outcome::Success(match req.content_type() {
            Some(t) => Self::from(t),
            None => Self::Raw,
        })
    }
}

trait AuthrorizationStrategy: Sized {
    fn authorize<'a, T: Iterator<Item = &'a str>>(auth_data: T) -> Result<Self>;
}

struct Authorization<S: AuthrorizationStrategy>(S);

#[derive(PartialEq)]
struct DummyAuth;

impl AuthrorizationStrategy for DummyAuth {
    fn authorize<'a, T: Iterator<Item = &'a str>>(auth_data: T) -> Result<Self> {
        Ok(Self)
    }
}

impl<'a, 'r, T> FromRequest<'a, 'r> for Authorization<T>
where
    T: AuthrorizationStrategy,
{
    type Error = anyhow::Error;

    fn from_request(request: &'a Request<'r>) -> Outcome<Self, Self::Error> {
        match T::authorize(request.headers().get("Authorization")) {
            Ok(auth) => Outcome::Success(Self(auth)),
            Err(e) => Outcome::Failure((Status::Unauthorized, anyhow!(format!("{}", e)))),
        }
    }
}

#[get("/<hash>")]
fn get_content(
    state: State<Store>,
    hash: MH,
    auth: Authorization<DummyAuth>,
) -> Result<Option<Stream<Cursor<Vec<u8>>>>> {
    match cas::ContentAddressedStorage::get(&state.deref(), hash.0) {
        Ok(Some(content)) => Ok(Some(Stream::chunked(Cursor::new(content), 1024))),
        Ok(None) => Ok(None),
        Err(e) => Err(e),
    }
}

#[post("/", data = "<data>")]
fn put_content(state: State<Store>, data: Data, codec: SupportedCodecs) -> Result<String> {
    match cas::ContentAddressedStorage::put(&state.deref(), data.open().take(STREAM_LIMIT), codec) {
        Ok(cid) => Ok(cid.to_string_of_base(Base::Base64Url)?),
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
