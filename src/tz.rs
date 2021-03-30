use anyhow::Result;
use bs58;
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
    pub cid: String,
}

// TODO comment on the query message format in KERI

impl FromStr for TZAuth {
    type Err = anyhow::Error;
    fn from_str<'a>(s: &'a str) -> Result<Self, Self::Err> {
        match tuple::<_, _, (), _>((
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
                    orbit: orbit_str.parse()?,
                    action: action_str.into(),
                    cid: cid_str.parse()?,
                })
            }
            Err(e) => Err(e.into()),
        }
    }
}

impl TZAuth {
    fn serialize_for_verification(&self) -> Vec<u8> {
        format!(
            "Tezos Signed Message: {} {}.kepler.net {} {} {}",
            &self.orbit, &self.timestamp, &self.pkh, &self.action, &self.cid
        )
        .into_bytes()
    }
}

#[tokio::main]
pub async fn verify(auth: &TZAuth) -> Result<()> {
    // get jwk
    let key = resolve_key(
        &format!("did:tz:{}#blockchainAccountId", &auth.pkh),
        &DIDTz::default(),
    )
    .await?;
    ssi::jws::verify_bytes(
        key.algorithm.unwrap(),
        &auth.serialize_for_verification(),
        &key,
        &bs58::decode(&auth.sig).into_vec()?,
    )?;
    Ok(())
}

#[test]
async fn simple_parse() {
    let sig_str = "Tezos Signed Message: uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA.kepler.net 2021-01-14T15:16:04Z <phk> GET uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA <sig>";
    let tza: TZAuth = sig_str.parse().unwrap();
}

#[test]
#[should_panic]
async fn simple_verify_fail() {
    let sig_str = "Tezos Signed Message: uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA.kepler.net 2021-01-14T15:16:04Z <phk> GET uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA <sig>";
    let tza: TZAuth = sig_str.parse().unwrap();

    verify(&tza).unwrap();
}

#[test]
async fn simple_verify_succeed() {
    let sig_str = "Tezos Signed Message: uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA.kepler.net 2021-01-14T15:16:04Z <phk> GET uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA <sig>";
    let tza: TZAuth = sig_str.parse().unwrap();

    verify(&tza).unwrap();
}
