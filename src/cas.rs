use super::codec::SupportedCodecs;
use anyhow::Result;
use libipld::cid::Cid;
use rocket::{
    form::{DataField, FromFormField},
    request::FromParam,
};
use std::str::FromStr;

#[derive(PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CidWrap(pub Cid);

// Orphan rule requires a wrapper type for this :(
impl<'a> FromParam<'a> for CidWrap {
    type Error = anyhow::Error;
    fn from_param(param: &'a str) -> Result<CidWrap> {
        Ok(CidWrap(Cid::from_str(param)?))
    }
}

#[rocket::async_trait]
impl<'r> FromFormField<'r> for CidWrap {
    async fn from_data(field: DataField<'r, '_>) -> rocket::form::Result<'r, Self> {
        Ok(CidWrap(
            field
                .name
                .source()
                .parse()
                .map_err(|_| field.unexpected())?,
        ))
    }
}

#[rocket::async_trait]
pub trait ContentAddressedStorage: Send + Sync {
    type Error;
    async fn put(&self, content: &[u8], codec: SupportedCodecs) -> Result<Cid, Self::Error>;
    async fn get(&self, address: &Cid) -> Result<Option<Vec<u8>>, Self::Error>;
    async fn delete(&self, address: &Cid) -> Result<(), Self::Error>;
    async fn list(&self) -> Result<Vec<Cid>, Self::Error>;
}
