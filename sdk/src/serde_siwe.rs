pub mod address {
    use hex::FromHex;
    use serde::de::{Deserialize, Deserializer, Error};

    pub fn deserialize<'de, D>(d: D) -> Result<[u8; 20], D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(d).and_then(|address| {
            <[u8; 20]>::from_hex(address.strip_prefix("0x").unwrap_or(&address))
                .map_err(|e| D::Error::custom(format!("failed to parse ethereum: {e}")))
        })
    }
}

pub mod signature {
    use hex::FromHex;
    use kepler_lib::cacaos::siwe_cacao::SIWESignature;
    use serde::de::{Deserialize, Deserializer, Error};

    pub fn deserialize<'de, D>(d: D) -> Result<SIWESignature, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(d).and_then(|sig| {
            <[u8; 65]>::from_hex(sig.strip_prefix("0x").unwrap_or(&sig))
                .map(Into::into)
                .map_err(|e| D::Error::custom(format!("failed to parse SIWE signature: {e}")))
        })
    }
}
