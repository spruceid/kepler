#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate tokio;

use anyhow::{anyhow, Error, Result};
use cid::multibase::Base;
use cid::Cid;
use rocket::{
    data::{ByteUnit, Data, ToByteUnit},
    form::Form,
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
use cas::{ContentAddressedStorage, CASDB};
use codec::{PutContent, SupportedCodecs};

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
    state: State<'_, Store<CASDB>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    auth: AuthToken,
) -> Result<Option<Stream<Cursor<Vec<u8>>>>, Debug<Error>> {
    match state.db.get(hash.0) {
        Ok(Some(content)) => Ok(Some(Stream::chunked(Cursor::new(content.to_owned()), 1024))),
        Ok(None) => Ok(None),
        Err(e) => Err(e)?,
    }
}

#[put("/<orbit_id>", format = "multipart/form-data", data = "<batch>")]
async fn batch_put_content(
    state: State<'_, Store<CASDB>>,
    orbit_id: CidWrap,
    batch: Form<Vec<PutContent>>,
    auth: AuthToken,
) -> Result<String, Debug<Error>> {
    todo!()
}

#[put("/<orbit_id>", data = "<data>")]
async fn put_content(
    state: State<'_, Store<CASDB>>,
    orbit_id: CidWrap,
    data: Data,
    codec: SupportedCodecs,
    auth: AuthToken,
) -> Result<String, Debug<Error>> {
    match state.db.put(
        Cursor::new(
            data.open(10.megabytes())
                .into_bytes() // TODO buffering 10Mb here is not wise, find a streaming way for this
                .await
                .map_err(|e| anyhow!(e))?
                .value,
        ),
        codec,
    ) {
        Ok(cid) => Ok(cid
            .to_string_of_base(Base::Base64Url)
            .map_err(|e| anyhow!(e))?),
        Err(e) => Err(e)?,
    }
}

#[delete("/<orbit_id>/<hash>")]
async fn delete_content(
    state: State<'_, Store<CASDB>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    auth: AuthToken,
) -> Result<(), Debug<Error>> {
    Ok(state.db.delete(hash.0)?)
}

#[launch]
fn rocket() -> rocket::Rocket {
    rocket::ignite()
        .manage(Store {
            db: CASDB::new(DB_PATH).unwrap(),
        })
        .mount(
            "/",
            routes![get_content, put_content, delete_content, batch_put_content],
        )
}
