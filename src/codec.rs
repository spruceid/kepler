use super::CidWrap;
use libipld::cid::Cid;
use rocket::{
    data::Data,
    data::{ByteUnit, Capped, DataStream, ToByteUnit},
    form::{DataField, FromForm, FromFormField, Result, ValueField},
    http::ContentType,
    request::{FromRequest, Outcome, Request},
};

#[derive(Clone, Copy)]
pub enum SupportedCodecs {
    Raw = 0x55,
    Json = 0x0200,
    MsgPack = 0x0201,
    Cbor = 0x51,
}

pub struct PutContent {
    pub codec: SupportedCodecs,
    // TODO dont use a Vec, but passing the datastream results in a hang
    pub content: Capped<Vec<u8>>,
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

#[rocket::async_trait]
impl<'r> FromRequest<'r> for SupportedCodecs {
    type Error = anyhow::Error;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(match req.content_type() {
            Some(t) => Self::from(t),
            None => Self::Raw,
        })
    }
}

#[rocket::async_trait]
impl<'r> FromFormField<'r> for PutContent {
    async fn from_data(field: DataField<'r, '_>) -> Result<'r, Self> {
        Ok(PutContent {
            codec: (&field.content_type).into(),
            content: field.data.open(1u8.megabytes()).into_bytes().await?,
        })
    }
}
