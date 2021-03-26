use rocket::{
    http::ContentType,
    request::{FromRequest, Outcome, Request},
};

pub enum SupportedCodecs {
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
