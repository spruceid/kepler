pub mod address {
    use hex::FromHex;
    use serde::de::{Deserialize, Deserializer, Error};

    pub fn deserialize<'de, D>(d: D) -> Result<[u8; 20], D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(d).and_then(|address| {
            <[u8; 20]>::from_hex(address.strip_prefix("0x").unwrap_or(&address))
                .map_err(|e| D::Error::custom(format!("failed to parse ethereum: {}", e)))
        })
    }
}

pub mod domain {
    use http::uri::Authority;
    use serde::de::{Deserialize, Deserializer, Error};

    pub fn deserialize<'de, D>(d: D) -> Result<Authority, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(d).and_then(|url_string| {
            url_string
                .try_into()
                .map_err(|e| D::Error::custom(format!("failed to parse domain: {}", e)))
        })
    }
}

pub mod optional_timestamp {
    use std::str::FromStr;

    use lib::siwe::TimeStamp;
    use serde::de::{Deserialize, Deserializer, Error};

    pub fn deserialize<'de, D>(d: D) -> Result<Option<TimeStamp>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(d)?
            .as_deref()
            .map(TimeStamp::from_str)
            .transpose()
            .map_err(|e| D::Error::custom(format!("failed to parse timestamp: {}", e)))
    }
}

pub mod timestamp {
    use std::str::FromStr;

    use lib::siwe::TimeStamp;
    use serde::de::{Deserialize, Deserializer, Error};

    pub fn deserialize<'de, D>(d: D) -> Result<TimeStamp, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(d).and_then(|timestamp_string| {
            TimeStamp::from_str(&timestamp_string)
                .map_err(|e| D::Error::custom(format!("failed to parse timestamp: {}", e)))
        })
    }
}

pub mod message {
    use std::str::FromStr;

    use lib::siwe::Message;
    use serde::{
        de::{Deserialize, Deserializer, Error as DeError},
        ser::{Serialize, Serializer},
    };

    pub fn deserialize<'de, D>(d: D) -> Result<Message, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(d).and_then(|unparsed| {
            Message::from_str(&unparsed)
                .map_err(|e| D::Error::custom(format!("failed to parse SIWE message: {}", e)))
        })
    }

    pub fn serialize<S>(msg: &Message, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        msg.to_string().serialize(s)
    }
}

pub mod signature {
    use hex::FromHex;
    use lib::cacaos::{BasicSignature, SIWESignature};
    use serde::de::{Deserialize, Deserializer, Error};

    pub fn deserialize<'de, D>(d: D) -> Result<BasicSignature<SIWESignature>, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(d)
            .and_then(|sig| {
                <[u8; 65]>::from_hex(sig.strip_prefix("0x").unwrap_or(&sig))
                    .map(Into::into)
                    .map_err(|e| D::Error::custom(format!("failed to parse SIWE signature: {}", e)))
            })
            .map(|s| BasicSignature { s })
    }
}
