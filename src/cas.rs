use anyhow::Result;
use kepler_lib::libipld::cid::Cid;
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
