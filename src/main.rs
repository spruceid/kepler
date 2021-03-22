#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;
extern crate multihash;

use rocket::{
    data::{Capped, Data, TempFile, ToByteUnit},
    response::NamedFile,
};

// 10 megabytes
const STREAM_LIMIT: usize = 10.megabytes();

struct MH(multihash::Multihash);

impl MH {
    pub fn read<R: std::io::Read>(r: R) -> Result<Self, multihash::Error> {
        Ok(Self(multihash::Multihash::read(r)?))
    }
}

// Orphan rule requires a wrapper type for this :(
impl<'a> rocket::request::FromParam<'_> for MH {
    type Error = multihash::Error;
    fn from_param(param: &'a str) -> Result<Self, multihash::Error> {
        Self::read(param.bytes())
    }
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
        .mount("/", routes![get_content])
        .mount("/", routes![put_content])
        .launch();
}
