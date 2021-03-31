use anyhow::Result;
use bs58::{decode, encode};
use did_tezos::DIDTz;
use nom::{
    bytes::complete::{tag, take_until},
    sequence::{preceded, tuple},
};
use serde_json;
use ssi::{
    did::{DIDMethod, Source},
    did_resolve::DIDResolver,
    did_resolve::ResolutionInputMetadata,
    jwk::{Algorithm, Base64urlUInt, OctetParams, Params, JWK},
    jws::verify_bytes,
    ldp::resolve_key,
};
use std::str::FromStr;

#[derive(Debug)]
pub struct TZAuth {
    pub sig: String,
    pub pk: String,
    pub pkh: String,
    pub timestamp: String,
    pub orbit: String,
    pub action: String,
    pub cid: String,
}

impl FromStr for TZAuth {
    type Err = anyhow::Error;
    fn from_str<'a>(s: &'a str) -> Result<Self, Self::Err> {
        match tuple::<_, _, (), _>((
            tag("Tezos Signed Message:"),                  // remove
            preceded(tag(" "), take_until(".kepler.net")), // get orbit
            tag(".kepler.net"),
            preceded(tag(" "), take_until(" ")), // get timestamp
            preceded(tag(" "), take_until(" ")), // get pk
            preceded(tag(" "), take_until(" ")), // get pkh
            preceded(tag(" "), take_until(" ")), // get action
            preceded(tag(" "), take_until(" ")), // get CID
            tag(" "),
        ))(s)
        {
            Ok((
                sig_str,
                (_, orbit_str, _, timestamp_str, pk_str, pkh_str, action_str, cid_str, _),
            )) => Ok(TZAuth {
                sig: sig_str.into(),
                pk: pk_str.into(),
                pkh: pkh_str.into(),
                timestamp: timestamp_str.into(),
                orbit: orbit_str.parse()?,
                action: action_str.into(),
                cid: cid_str.parse()?,
            }),
            Err(e) => Err(e.into()),
        }
    }
}

impl TZAuth {
    fn serialize_for_verification(&self) -> Vec<u8> {
        format!(
            "Tezos Signed Message: {}.kepler.net {} {} {} {} {}",
            &self.orbit, &self.timestamp, &self.pk, &self.pkh, &self.action, &self.cid
        )
        .into_bytes()
    }
}

impl core::fmt::Display for TZAuth {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Tezos Signed Message: {}.kepler.net {} {} {} {} {} {}",
            &self.orbit, &self.timestamp, &self.pk, &self.pkh, &self.action, &self.cid, &self.sig
        )
    }
}

pub fn verify(auth: &TZAuth) -> Result<()> {
    let alg = match &auth.pkh.as_str()[..3] {
        "tz1" => Algorithm::EdDSA,
        "tz2" => Algorithm::ES256KR,
        "tz3" => Algorithm::PS256,
        _ => return Err(anyhow!("Invalid Public Key Hash, {}", &auth.pkh)),
    };
    verify_bytes(
        alg,
        &auth.serialize_for_verification(),
        &JWK {
            public_key_use: None,
            key_operations: None,
            algorithm: Some(alg),
            key_id: None,
            x509_url: None,
            x509_certificate_chain: None,
            x509_thumbprint_sha1: None,
            x509_thumbprint_sha256: None,
            params: match alg {
                Algorithm::EdDSA => Params::OKP(OctetParams {
                    curve: "Ed25519".into(),
                    // TODO the slicing must happen on the bytes, not the characters
                    public_key: Base64urlUInt(decode(&auth.pk[4..]).with_check(None).into_vec()?),
                    private_key: None,
                }),
                Algorithm::ES256KR => todo!(),
                Algorithm::PS256 => todo!(),
                _ => return Err(anyhow!("Invalid Public Key Hash, {}", &auth.pkh)),
            },
        },
        // TODO the slicing must happen on the bytes, not the characters
        &decode(&auth.sig[5..]).with_check(None).into_vec()?,
    )?;
    Ok(())
}

#[test]
async fn simple_parse() {
    let auth_str = "Tezos Signed Message: uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA.kepler.net 2021-01-14T15:16:04Z edpkN2QDv7TGEPfAwzs9qCujsB1CxtVSjeesSj7EfFQh5cj4PJiH9 tz1ZoKiKMuSEyQ9JTETx7ZTwmnRtCxXoxduN GET uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA edsigWUz8sUVeqTJgXW7SMzihcmJ2JPQxPx9T5G6hx6P2yJSs9gYQSDNLFEm3rPYVB8fajgRS6qqAEX4LHhUCuaucp1qKHxpU5";
    let tza: TZAuth = auth_str.parse().unwrap();
}

#[test]
#[should_panic]
async fn simple_verify_fail() {
    let auth_str = "Tezos Signed Message: uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFKjqY6zejzKoA.kepler.net 2021-01-14T15:16:04Z edpkN2QDv7TGEPfAwzs9qCujsB1CxtVSjeesSj7EfFQh5cj4PJiH9 tz1ZoKiKMuSEyQ9JTETx7ZTwmnRtCxXoxduN GET uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA edsigWUz8sUVeqTJgXW7SMzihcmJ2JPQxPx9T5G6hx6P2yJSs9gYQSDNLFEm3rPYVB8fajgRS6qqAEX4LHhUCuaucp1qKHxpU5";
    let tza: TZAuth = auth_str.parse().unwrap();

    verify(&tza).unwrap();
}

#[test]
async fn simple_verify_succeed() {
    let auth_str = "Tezos Signed Message: uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA.kepler.net 2021-01-14T15:16:04Z edpk2LN8RVL7BEkNpugYQGgAXDTx2Nu3rYkKUxuLkXwLsRSpHMr56N tz1T1QsjyS9jkhuRW2xTnXXw1yas1z2oUKBk PUT uAYAEHiDoN2Q6QgzD6zqWuvgFoUj130OydcuzWRl8b5q5TpWuIg edsigHyi8nwAKFvECEcUzs9PNBmEA68tNdAKa762aqsMX7gcPVwnfCrhcxtYKBNid17QSygQKfVuJgx8CtVuVB3tsACsfFvUXg";
    let tza: TZAuth = auth_str.parse().unwrap();

    verify(&tza).unwrap();
}

#[test]
async fn round_trip() {
    let ts = "2021-01-14T15:16:04Z";
    let dummy_cid = "uAYAEHiB0uGRNPXEMdA9L-lXR2MKIZzKlgW1z6Ug4fSv3LRSPfQ";
    let dummy_orbit = "uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA";
    let j = JWK::generate_ed25519().unwrap();
    let did = DIDTz::default().generate(&Source::Key(&j)).unwrap();
    let pkh = did.split(":").last().unwrap();
    let pk: String = format!(
        "edpk{}",
        match &j.params {
            Params::OKP(p) => bs58::encode(&p.public_key.0).with_check().into_string(),
            _ => panic!(),
        }
    );
    let message = format!(
        "Tezos Signed Message: {}.kepler.net {} {} {} GET {}",
        &dummy_orbit, &ts, &pk, &pkh, &dummy_cid
    );
    let sig_bytes = ssi::jws::sign_bytes(Algorithm::EdDSA, &message.as_bytes(), &j).unwrap();
    let sig = format!(
        "edsig{}",
        bs58::encode(&sig_bytes).with_check().into_string()
    );
    let tz = TZAuth {
        sig,
        pk,
        pkh: pkh.into(),
        timestamp: ts.into(),
        orbit: dummy_orbit.into(),
        action: "GET".into(),
        cid: dummy_cid.into(),
    };
    assert_eq!(message.as_bytes(), tz.serialize_for_verification());
    assert!(verify(&tz).is_ok());
}
