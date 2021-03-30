use anyhow::Result;
use cid::Cid;
use did_tezos::DIDTz;
use nom::{
    bytes::complete::{tag, take_until},
    sequence::{preceded, tuple},
};
use ssi::{
    jwk::{Algorithm, JWK},
    jws::verify_bytes,
    ldp::resolve_key,
};
use std::str::FromStr;
use tokio;

pub struct TZAuth {
    pub sig: String,
    pub pkh: String,
    pub timestamp: String,
    pub orbit: String,
    pub action: String,
    pub cid: Cid,
}

impl FromStr for TZAuth {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match tuple((
            tag("Tezos Signed Message:"),                  // remove
            preceded(tag(" "), take_until(".kepler.net")), // get orbit
            tag(".kepler.net"),
            preceded(tag(" "), take_until(" ")), // get timestamp
            preceded(tag(" "), take_until(" ")), // get pkh
            preceded(tag(" "), take_until(" ")), // get action
            preceded(tag(" "), take_until(" ")), // get CID
            tag(" "),
        ))(s)
        {
            Ok((sig_str, (_, orbit_str, _, timestamp_str, pkh_str, action_str, cid_str, _))) => {
                Ok(TZAuth {
                    sig: sig_str.into(),
                    pkh: pkh_str.into(),
                    timestamp: timestamp_str.into(),
                    orbit: orbit_str.into(),
                    action: action_str.into(),
                    cid: cid_str.parse()?,
                })
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[tokio::main]
pub async fn verify(auth: TZAuth) -> Result<()> {
    // get jwk
    let key = resolve_key(
        &format!("did:tz:{}#blockchainAccountId", &auth.pkh),
        &DIDTz::default(),
    )
    .await?;
    // match key type to determine tz1/2/3
    match (key.algorithm, auth.pkh.as_str()) {
        (Some(Algorithm::EdDSA), "tz1") => Ok(()),   // tz1
        (Some(Algorithm::ES256KR), "tz2") => Ok(()), // tz2
        (Some(Algorithm::PS256), "tz3") => Ok(()),   // tz3
        _ => Err(anyhow!("Invalid Key Type")),
    }
    //
    // verify according to tz key edition
    // Ok(verify_bytes(
    //     Algorithm::EdDSA,
    //     auth.data.as_bytes(),

    //     auth.sig.as_bytes(),
    // )?)
}
