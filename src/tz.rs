use anyhow::Result;
use bs58;
use did_tezos::DIDTz;
use hex;
use nom::{
    bytes::complete::{tag, take_until},
    sequence::{preceded, tuple},
};
use serde_json;
use ssi::{
    did::{DIDMethod, Source},
    did_resolve::DIDResolver,
    did_resolve::ResolutionInputMetadata,
    jwk::{ECParams, Algorithm, Base64urlUInt, OctetParams, Params, JWK},
    jws::verify_bytes,
    ldp::resolve_key,
};
use std::str::FromStr;
use multihash::{Code, MultihashDigest};

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
        let message = format!(
                "Tezos Signed Message: {}.kepler.net {} {} {} {} {}",
                &self.orbit, &self.timestamp, &self.pk, &self.pkh, &self.action, &self.cid
            );
        Code::Blake2b256.digest(&encode_string(&message)).digest().to_vec()
    }
}

fn encode_string(s: &str) -> Vec<u8> {
    hex::decode(format!("0501{:08x}{}", &s.as_bytes().len(), &hex::encode(&s.as_bytes())).into_bytes()).unwrap()
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
    let key = from_tezos_key(&auth.pk)?;
    verify_bytes(
        key.algorithm.ok_or(anyhow!("Invalid Signature Scheme"))?,
        &auth.serialize_for_verification(),
        &key,
        &bs58::decode(&auth.sig).with_check(None).into_vec()?[5..].to_owned(),
    )?;
    Ok(())
}

pub fn from_tezos_key(tz_pk: &str) -> Result<JWK> {
    let (alg, params) = match &tz_pk[..4] {
        "edpk" => (
            Algorithm::EdDSA,
            Params::OKP(OctetParams {
                curve: "Ed25519".into(),
                public_key: Base64urlUInt(
                    bs58::decode(&tz_pk).with_check(None).into_vec()?[4..].to_owned(),
                ),
                private_key: None,
            })
        ),
        "sppk" => (
            Algorithm::ES256KR,
            Params::EC(ECParams {
                curve: Some("secp256k1".into()),
                // TODO
                x_coordinate: None,
                y_coordinate: None,
                ecc_private_key: None
            })
        ),
        "p2pk" => (
            Algorithm::PS256,
            Params::EC(ECParams {
                curve: Some("P-256".into()),
                // TODO
                x_coordinate: None,
                y_coordinate: None,
                ecc_private_key: None
            })
        ),
        _ => Err(anyhow!("Invalid Tezos Public Key"))?
    };
    Ok(JWK {
            public_key_use: None,
            key_operations: None,
            algorithm: Some(alg),
            key_id: None,
            x509_url: None,
            x509_certificate_chain: None,
            x509_thumbprint_sha1: None,
            x509_thumbprint_sha256: None,
            params
    })
}

#[test]
async fn string_encoding() {
    assert_eq!(&encode_string("message"), &[0x05, 0x01, 0x00, 0x00, 0x00, 0x07, 0x6d, 0x65, 0x73, 0x73, 0x61, 0x67, 0x65])
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
    let auth_str = "Tezos Signed Message: uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA.kepler.net 1617729172025 edpkuthnQ7YdexSxGEHYSbrweH31Zd75roc7W42Lgt8LJM8PX4sX6m tz1WWXeGFgtARRLPPzT2qcpeiQZ8oQb6rBZd GET uAYAEHiB0uGRNPXEMdA9L-lXR2MKIZzKlgW1z6Ug4fSv3LRSPfQ edsigu1XepfKcX2ec5Cn8pXxXSA3mX2ygWm5akw8bJgnNDDFQpAevK2vDxXfzL1gidopuHfkDci72Z7YahrZ7jaW8akgwGhR7Fc";
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
    let pk: String = match &j.params {
        Params::OKP(p) => bs58::encode(
            [13, 15, 37, 217]
                .iter()
                .chain(&p.public_key.0)
                .map(|&x| x)
                .collect::<Vec<u8>>(),
        )
        .with_check()
        .into_string(),
        _ => panic!(),
    };
    let tz_unsigned = TZAuth {
        sig: "".into(),
        pk,
        pkh: pkh.into(),
        timestamp: ts.into(),
        orbit: dummy_orbit.into(),
        action: "PUT".into(),
        cid: dummy_cid.into(),
    };
    let message = tz_unsigned.serialize_for_verification();
    let sig_bytes = ssi::jws::sign_bytes(Algorithm::EdDSA, &message, &j).unwrap();
    let sig = bs58::encode(
        [9, 245, 205, 134, 18]
            .iter()
            .chain(&sig_bytes)
            .map(|&x| x)
            .collect::<Vec<u8>>(),
    )
    .with_check()
    .into_string();
    let tz = TZAuth { sig, ..tz_unsigned };

    assert_eq!(message, tz.serialize_for_verification());
    assert!(verify(&tz).is_ok());
}
