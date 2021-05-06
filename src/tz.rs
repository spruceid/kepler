use crate::auth::{Action, AuthorizationPolicy, AuthorizationToken};
use anyhow::Result;
use bs58;
use hex;
use libipld::{
    cid::multibase::Base,
    multihash::{Code, MultihashDigest},
    Cid,
};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    combinator::map_parser,
    multi::many1,
    sequence::{delimited, preceded, tuple},
    IResult, ParseTo,
};
use ssi::{
    jwk::{Algorithm, Base64urlUInt, ECParams, OctetParams, Params, JWK},
    jws::verify_bytes,
};
use std::str::FromStr;

#[derive(Debug)]
pub struct TZAuth {
    pub sig: String,
    pub pk: String,
    pub pkh: String,
    pub timestamp: String,
    pub action: Action,
}

impl FromStr for TZAuth {
    type Err = anyhow::Error;
    fn from_str<'a>(s: &'a str) -> Result<Self, Self::Err> {
        match tuple::<_, _, nom::error::Error<&'a str>, _>((
            tag("Tezos Signed Message: kepler.net"), // remove
            space_delimit,                           // get timestamp
            space_delimit,                           // get pk
            space_delimit,                           // get pkh
            tag(" "),
            parse_action, // get action
            tag(" "),
        ))(s)
        {
            Ok((sig_str, (_, timestamp_str, pk_str, pkh_str, _, action, _))) => Ok(TZAuth {
                sig: sig_str.into(),
                pk: pk_str.into(),
                pkh: pkh_str.into(),
                timestamp: timestamp_str.into(),
                action,
            }),
            // TODO there is a lifetime issue which prevents using the nom error here
            Err(_) => Err(anyhow!("TzAuth Parsing Failed")),
        }
    }
}

fn space_delimit<'a>(s: &'a str) -> IResult<&str, &str> {
    preceded(tag(" "), take_until(" "))(s)
}

// NOTE this will consume the whole string, it should only be called on fragments which are already separated
fn parse_cid<'a>(s: &'a str) -> IResult<&str, Cid> {
    s.parse_to()
        .ok_or(nom::Err::Failure(nom::error::make_error(
            s,
            nom::error::ErrorKind::IsNot,
        )))
        .map(|cid| ("", cid))
}

fn parse_get<'a>(s: &'a str) -> IResult<&str, Action> {
    tuple((
        map_parser(take_until(" "), parse_cid),
        tag(" GET"),
        many1(map_parser(space_delimit, parse_cid)),
    ))(s)
    .map(|(rest, (orbit_id, _, content))| (rest, Action::Get { orbit_id, content }))
}

fn parse_put<'a>(s: &'a str) -> IResult<&str, Action> {
    tuple((
        map_parser(take_until(" "), parse_cid),
        tag(" PUT"),
        many1(map_parser(space_delimit, parse_cid)),
    ))(s)
    .map(|(rest, (orbit_id, _, content))| (rest, Action::Put { orbit_id, content }))
}

fn parse_del<'a>(s: &'a str) -> IResult<&str, Action> {
    tuple((
        map_parser(take_until(" "), parse_cid),
        tag(" DEL"),
        many1(map_parser(space_delimit, parse_cid)),
    ))(s)
    .map(|(rest, (orbit_id, _, content))| (rest, Action::Del { orbit_id, content }))
}

fn parse_create<'a>(s: &'a str) -> IResult<&str, Action> {
    tuple((
        map_parser(take_until(" "), parse_cid),
        tag(" CREATE"),
        space_delimit, // salt (orbit secret + nonce)
        many1(map_parser(space_delimit, parse_cid)),
    ))(s)
    .map(|(rest, (orbit_id, _, salt, content))| {
        (
            rest,
            Action::Create {
                orbit_id,
                content,
                salt: salt.into(),
            },
        )
    })
}

fn parse_action<'a>(s: &'a str) -> IResult<&str, Action> {
    alt((parse_get, parse_put, parse_del, parse_create))(s)
}

fn serialize_action(action: &Action) -> Result<String> {
    match action {
        Action::Put { orbit_id, content } => serialize_content_action("PUT", orbit_id, content),
        Action::Get { orbit_id, content } => serialize_content_action("GET", orbit_id, content),
        Action::Del { orbit_id, content } => serialize_content_action("DEL", orbit_id, content),
        Action::Create {
            orbit_id,
            content,
            salt,
        } => Ok([
            &orbit_id.to_string_of_base(Base::Base64Url)?,
            "CREATE",
            &salt,
            &content
                .iter()
                .map(|c| c.to_string_of_base(Base::Base64Url))
                .collect::<Result<Vec<String>, libipld::cid::Error>>()?
                .join(" "),
        ]
        .join(" ")),
    }
}

fn serialize_content_action(action: &str, orbit_id: &Cid, content: &[Cid]) -> Result<String> {
    Ok([
        &orbit_id.to_string_of_base(Base::Base64Url)?,
        action,
        &content
            .iter()
            .map(|c| c.to_string_of_base(Base::Base64Url))
            .collect::<Result<Vec<String>, libipld::cid::Error>>()?
            .join(" "),
    ]
    .join(" "))
}

impl TZAuth {
    fn serialize_for_verification(&self) -> Result<Vec<u8>> {
        let message = format!(
            "Tezos Signed Message: kepler.net {} {} {}",
            &self.timestamp,
            serialize_action(&self.action)?,
            &self.pk
        );
        Ok(Code::Blake2b256
            .digest(&encode_string(&message))
            .digest()
            .to_vec())
    }
}

impl AuthorizationToken for TZAuth {
    const HEADER_KEY: &'static str = "Authorization";
    type Policy = TezosBasicAuthorization;

    fn extract<'a, T: Iterator<Item = &'a str>>(auth_data: T) -> Result<Self> {
        todo!()
    }

    fn action(&self) -> &Action {
        &self.action
    }
}

fn encode_string(s: &str) -> Vec<u8> {
    hex::decode(
        format!(
            "0501{:08x}{}",
            &s.as_bytes().len(),
            &hex::encode(&s.as_bytes())
        )
        .into_bytes(),
    )
    .unwrap()
}

impl core::fmt::Display for TZAuth {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Tezos Signed Message: kepler.net {} {} {} {} {}",
            &self.timestamp,
            &self.pk,
            &self.pkh,
            serialize_action(&self.action).map_err(|_| core::fmt::Error)?,
            &self.sig
        )
    }
}

pub fn verify(auth: &TZAuth) -> Result<()> {
    let key = from_tezos_key(&auth.pk)?;
    verify_bytes(
        key.algorithm.ok_or(anyhow!("Invalid Signature Scheme"))?,
        &auth.serialize_for_verification()?,
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
            }),
        ),
        "sppk" => (
            Algorithm::ES256KR,
            Params::EC(ECParams {
                curve: Some("secp256k1".into()),
                // TODO
                x_coordinate: None,
                y_coordinate: None,
                ecc_private_key: None,
            }),
        ),
        "p2pk" => (
            Algorithm::PS256,
            Params::EC(ECParams {
                curve: Some("P-256".into()),
                // TODO
                x_coordinate: None,
                y_coordinate: None,
                ecc_private_key: None,
            }),
        ),
        _ => Err(anyhow!("Invalid Tezos Public Key"))?,
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
        params,
    })
}

pub struct TezosBasicAuthorization;

#[rocket::async_trait]
impl AuthorizationPolicy for TezosBasicAuthorization {
    type Token = TZAuth;

    async fn authorize<'a>(&self, auth_token: &'a Self::Token) -> Result<&'a Action> {
        verify(auth_token).map(|_| auth_token.action())
    }
}

#[test]
async fn string_encoding() {
    assert_eq!(
        &encode_string("message"),
        &[0x05, 0x01, 0x00, 0x00, 0x00, 0x07, 0x6d, 0x65, 0x73, 0x73, 0x61, 0x67, 0x65]
    )
}

#[test]
async fn simple_parse() {
    let auth_str = "Tezos Signed Message: kepler.net 2021-01-14T15:16:04Z edpkurFSehqm2HhLP9sZ4ZRW5nLZgyWErW8wYxgEUPHCMCy6Hk1tbm tz1Y6SXe4J9DBVuGM3GnWC2jnmDkA6fBVyjg uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA PUT uAYAEHiB0uGRNPXEMdA9L-lXR2MKIZzKlgW1z6Ug4fSv3LRSPfQ edsigtmZ5tgugBSKjBJgptkm523C9EtVWrBhLYtv9MTAE6qF6mii2mFapdQfcCMsVzRisgQ3Nx61qC9Ut3VigyEC1s19RLwgkog";
    let _: TZAuth = auth_str.parse().unwrap();
}

#[test]
#[should_panic]
async fn simple_verify_fail() {
    let auth_str = "Tezos Signed Message: kepler.net 2021-01-14T15:15:04Z edpkurFSehqm2HhLP9sZ4ZRW5nLZgyWErW8wYxgEUPHCMCy6Hk1tbm tz1Y6SXe4J9DBVuGM3GnWC2jnmDkA6fBVyjg uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA PUT uAYAEHiB0uGRNPXEMdA9L-lXR2MKIZzKlgW1z6Ug4fSv3LRSPfQ edsigtmZ5tgugBSKjBJgptkm523C9EtVWrBhLYtv9MTAE6qF6mii2mFapdQfcCMsVzRisgQ3Nx61qC9Ut3VigyEC1s19RLwgkog";
    let tza: TZAuth = auth_str.parse().unwrap();

    verify(&tza).unwrap();
}

#[test]
async fn simple_verify_succeed() {
    let auth_str = "Tezos Signed Message: kepler.net 2021-01-14T15:16:04Z edpkurFSehqm2HhLP9sZ4ZRW5nLZgyWErW8wYxgEUPHCMCy6Hk1tbm tz1Y6SXe4J9DBVuGM3GnWC2jnmDkA6fBVyjg uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA PUT uAYAEHiB0uGRNPXEMdA9L-lXR2MKIZzKlgW1z6Ug4fSv3LRSPfQ edsigtmZ5tgugBSKjBJgptkm523C9EtVWrBhLYtv9MTAE6qF6mii2mFapdQfcCMsVzRisgQ3Nx61qC9Ut3VigyEC1s19RLwgkog";
    let tza: TZAuth = auth_str.parse().unwrap();

    verify(&tza).unwrap();
}

#[test]
async fn round_trip() {
    use did_pkh::DIDPKH;
    use ssi::did::{DIDMethod, Source};

    let ts = "2021-01-14T15:16:04Z";
    let dummy_cid = "uAYAEHiB0uGRNPXEMdA9L-lXR2MKIZzKlgW1z6Ug4fSv3LRSPfQ";
    let dummy_orbit = "uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA";
    let j = JWK::generate_ed25519().unwrap();
    let did = DIDPKH.generate(&Source::KeyAndPattern(&j, "tz")).unwrap();
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
        action: Action::Put {
            orbit_id: Cid::from_str(dummy_orbit).expect("failed to parse orbit ID"),
            content: vec![Cid::from_str(dummy_cid).expect("failed to parse CID")],
        },
    };
    let message = tz_unsigned
        .serialize_for_verification()
        .expect("failed to serialize authz message");
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

    assert_eq!(
        message,
        tz.serialize_for_verification()
            .expect("failed to serialize authz message")
    );
    assert!(verify(&tz).is_ok());
}
