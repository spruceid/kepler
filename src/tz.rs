use crate::auth::{Action, AuthorizationPolicy, AuthorizationToken};
use anyhow::Result;
use libipld::{cid::multibase::Base, Cid};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    combinator::map_parser,
    multi::many1,
    sequence::{preceded, tuple},
    IResult, ParseTo,
};
use rocket::request::{FromRequest, Outcome, Request};
use ssi::{
    did::DIDURL,
    jws::verify_bytes,
    tzkey::{decode_tzsig, jwk_from_tezos_key},
};
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct TezosAuthorizationString {
    pub sig: String,
    pub domain: String,
    pub pk: String,
    pub pkh: String,
    pub timestamp: String,
    pub action: Action,
}

impl FromStr for TezosAuthorizationString {
    type Err = anyhow::Error;
    fn from_str<'a>(s: &'a str) -> Result<Self, Self::Err> {
        match tuple::<_, _, nom::error::Error<&'a str>, _>((
            tag("Tezos Signed Message:"), // remove
            space_delimit,                // domain string
            space_delimit,                // get timestamp
            space_delimit,                // get pk
            space_delimit,                // get pkh
            tag(" "),
            parse_action, // get action
            tag(" "),
        ))(s)
        {
            Ok((sig_str, (_, domain_str, timestamp_str, pk_str, pkh_str, _, action, _))) => {
                Ok(TezosAuthorizationString {
                    sig: sig_str.into(),
                    domain: domain_str.into(),
                    pk: pk_str.into(),
                    pkh: pkh_str.into(),
                    timestamp: timestamp_str.into(),
                    action,
                })
            }
            // TODO there is a lifetime issue which prevents using the nom error here
            Err(_) => Err(anyhow!("TzAuth Parsing Failed")),
        }
    }
}

fn space_delimit(s: &str) -> IResult<&str, &str> {
    preceded(tag(" "), take_until(" "))(s)
}

// NOTE this will consume the whole string, it should only be called on fragments which are already separated
fn parse_cid(s: &str) -> IResult<&str, Cid> {
    s.parse_to()
        .ok_or_else(|| nom::Err::Failure(nom::error::make_error(s, nom::error::ErrorKind::IsNot)))
        .map(|cid| ("", cid))
}

fn parse_list(s: &str) -> IResult<&str, Action> {
    tuple((map_parser(take_until(" "), parse_cid), tag(" LIST")))(s)
        .map(|(rest, (orbit_id, _))| (rest, Action::List { orbit_id }))
}

fn parse_get(s: &str) -> IResult<&str, Action> {
    tuple((
        map_parser(take_until(" "), parse_cid),
        tag(" GET"),
        many1(map_parser(space_delimit, parse_cid)),
    ))(s)
    .map(|(rest, (orbit_id, _, content))| (rest, Action::Get { orbit_id, content }))
}

fn parse_put(s: &str) -> IResult<&str, Action> {
    tuple((
        map_parser(take_until(" "), parse_cid),
        tag(" PUT"),
        many1(map_parser(space_delimit, parse_cid)),
    ))(s)
    .map(|(rest, (orbit_id, _, content))| (rest, Action::Put { orbit_id, content }))
}

fn parse_del(s: &str) -> IResult<&str, Action> {
    tuple((
        map_parser(take_until(" "), parse_cid),
        tag(" DEL"),
        many1(map_parser(space_delimit, parse_cid)),
    ))(s)
    .map(|(rest, (orbit_id, _, content))| (rest, Action::Del { orbit_id, content }))
}

fn parse_create(s: &str) -> IResult<&str, Action> {
    tuple((
        map_parser(take_until(" "), parse_cid),
        tag(" CREATE"),
        space_delimit, // parameters
        many1(map_parser(space_delimit, parse_cid)),
    ))(s)
    .map(|(rest, (orbit_id, _, params, content))| {
        (
            rest,
            Action::Create {
                orbit_id,
                content,
                parameters: params.into(),
            },
        )
    })
}

fn parse_action(s: &str) -> IResult<&str, Action> {
    alt((parse_get, parse_put, parse_del, parse_create, parse_list))(s)
}

fn serialize_action(action: &Action) -> Result<String> {
    match action {
        Action::Put { orbit_id, content } => serialize_content_action("PUT", orbit_id, content),
        Action::Get { orbit_id, content } => serialize_content_action("GET", orbit_id, content),
        Action::Del { orbit_id, content } => serialize_content_action("DEL", orbit_id, content),
        Action::List { orbit_id } => serialize_content_action("LIST", orbit_id, &[]),
        Action::Create {
            orbit_id,
            content,
            parameters,
        } => Ok([
            &orbit_id.to_string_of_base(Base::Base58Btc)?,
            "CREATE",
            &parameters,
            &content
                .iter()
                .map(|c| c.to_string_of_base(Base::Base58Btc))
                .collect::<Result<Vec<String>, libipld::cid::Error>>()?
                .join(" "),
        ]
        .join(" ")),
    }
}

fn serialize_content_action(action: &str, orbit_id: &Cid, content: &[Cid]) -> Result<String> {
    Ok([
        &orbit_id.to_string_of_base(Base::Base58Btc)?,
        action,
        &content
            .iter()
            .map(|c| c.to_string_of_base(Base::Base58Btc))
            .collect::<Result<Vec<String>, libipld::cid::Error>>()?
            .join(" "),
    ]
    .join(" "))
}

impl TezosAuthorizationString {
    fn serialize_for_verification(&self) -> Result<Vec<u8>> {
        let message = format!(
            "Tezos Signed Message: {} {} {} {} {}",
            &self.domain,
            &self.timestamp,
            &self.pk,
            &self.pkh,
            serialize_action(&self.action)?
        );
        Ok(encode_string(&message))
    }

    fn verify(&self) -> Result<()> {
        let key = jwk_from_tezos_key(&self.pk)?;
        let (_, sig) = decode_tzsig(&self.sig)?;
        Ok(verify_bytes(
            key.algorithm
                .ok_or_else(|| anyhow!("Invalid Signature Scheme"))?,
            &self.serialize_for_verification()?,
            &key,
            &sig,
        )?)
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for TezosAuthorizationString {
    type Error = anyhow::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match request
            .headers()
            .get_one("Authorization")
            .map(|s| Self::from_str(s))
        {
            Some(Ok(t)) => Outcome::Success(t),
            _ => Outcome::Forward(()),
        }
    }
}

impl AuthorizationToken for TezosAuthorizationString {
    fn action(&self) -> Action {
        self.action.clone()
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

impl core::fmt::Display for TezosAuthorizationString {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Tezos Signed Message: {} {} {} {} {} {}",
            &self.domain,
            &self.timestamp,
            &self.pk,
            &self.pkh,
            serialize_action(&self.action).map_err(|_| core::fmt::Error)?,
            &self.sig
        )
    }
}

#[derive(Clone)]
pub struct TezosBasicAuthorization {
    pub controllers: Vec<DIDURL>,
}

#[rocket::async_trait]
impl AuthorizationPolicy for TezosBasicAuthorization {
    type Token = TezosAuthorizationString;

    async fn authorize<'a>(&self, auth_token: &'a Self::Token) -> Result<()> {
        let requester = DIDURL {
            did: format!("did:pkh:tz:{}", &auth_token.pkh),
            fragment: Some("TezosMethod2021".to_string()),
            ..Default::default()
        };

        if !self.controllers.contains(&requester) {
            Err(anyhow!("Requester not a controller of the orbit"))
        } else {
            Ok(())
        }
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
    let _: TezosAuthorizationString = auth_str.parse().unwrap();
}

#[test]
#[should_panic]
async fn simple_verify_fail() {
    let auth_str = "Tezos Signed Message: kepler.net 2021-01-14T15:15:04Z edpkurFSehqm2HhLP9sZ4ZRW5nLZgyWErW8wYxgEUPHCMCy6Hk1tbm tz1Y6SXe4J9DBVuGM3GnWC2jnmDkA6fBVyjg uAYAEHiB_A0nLzANfXNkW5WCju51Td_INJ6UacFK7qY6zejzKoA PUT uAYAEHiB0uGRNPXEMdA9L-lXR2MKIZzKlgW1z6Ug4fSv3LRSPfQ edsigtmZ5tgugBSKjBJgptkm523C9EtVWrBhLYtv9MTAE6qF6mii2mFapdQfcCMsVzRisgQ3Nx61qC9Ut3VigyEC1s19RLwgkog";
    let tza: TezosAuthorizationString = auth_str.parse().unwrap();
    tza.verify().unwrap();
}

#[test]
async fn simple_verify_succeed() {
    let auth_str = "Tezos Signed Message: kepler.net 2021-01-14T15:16:04Z edpkvR6aAd12AiVyh9UJUntyNtLM2usrpCq4avG8KozdSMU7vvu7qU tz1c5eqtBarJ9brFnmUy2n7Bvg1RCpWk29eU z3v8BBKAxmb5DPsoCsaucZZ26FzPSbLWDAGtpHSiKjA4AJLQ3my PUT z3v8BBKAGbGkuFU8TQq3J7k9XDs9udtMCic4KMS6HBxHczS1Tyv edsigtmtSYSCCB8yyvj3BiFHSgbS21UAoR2jrQMWEr3eA99Czyph3duzEVAVYnG8chBKBPMhxD9ZyTwQSAoMuGr6bnotP9m9wvK";
    let tza: TezosAuthorizationString = auth_str.parse().unwrap();
    tza.verify().unwrap();
}

#[test]
async fn round_trip() {
    use did_pkh::DIDPKH;
    use ssi::{
        did::{DIDMethod, Source},
        jwk::{Algorithm, Params, JWK},
    };

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
    let tz_unsigned = TezosAuthorizationString {
        sig: "".into(),
        domain: "kepler.net".into(),
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
    let sig_bytes = ssi::jws::sign_bytes(Algorithm::EdBlake2b, &message, &j).unwrap();
    let sig = bs58::encode(
        [9, 245, 205, 134, 18]
            .iter()
            .chain(&sig_bytes)
            .map(|&x| x)
            .collect::<Vec<u8>>(),
    )
    .with_check()
    .into_string();
    let tz = TezosAuthorizationString { sig, ..tz_unsigned };

    assert_eq!(
        message,
        tz.serialize_for_verification()
            .expect("failed to serialize authz message")
    );
    assert!(tz.verify().is_ok());
}
