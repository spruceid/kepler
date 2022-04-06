use crate::{
    auth::{AuthorizationPolicy, AuthorizationToken},
    orbit::Orbit,
    manifest::Manifest,
    resource::ResourceId,
    capabilities::{Invoke, AuthRef}
};
use anyhow::Result;
use nom::{
    bytes::complete::{tag, take_until},
    sequence::{preceded, tuple},
    IResult,
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
    pub target: ResourceId,
    pub delegate: Option<String>,
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
            space_delimit,                // get target
            parse_delegate,               // get delegate
            tag(" "),
        ))(s)
        {
            Ok((
                sig_str,
                (_, domain_str, timestamp_str, pk_str, pkh_str, target_str, delegate_str, _),
            )) => Ok(TezosAuthorizationString {
                sig: sig_str.into(),
                domain: domain_str.into(),
                pk: pk_str.into(),
                pkh: pkh_str.into(),
                timestamp: timestamp_str.into(),
                target: target_str.parse()?,
                delegate: delegate_str.map(|s| s.to_string()),
            }),
            // TODO there is a lifetime issue which prevents using the nom error here
            Err(_) => Err(anyhow!("TzAuth Parsing Failed")),
        }
    }
}

fn parse_delegate(s: &str) -> IResult<&str, Option<&str>> {
    if s.starts_with("did:") {
        space_delimit(s).map(|(r, s)| (r, Some(s)))
    } else {
        Ok((s, None))
    }
}

fn space_delimit(s: &str) -> IResult<&str, &str> {
    preceded(tag(" "), take_until(" "))(s)
}

impl TezosAuthorizationString {
    pub fn serialize(&self) -> Result<String> {
        let mut s = format!(
            "Tezos Signed Message: {} {} {} {} {}",
            &self.domain, &self.timestamp, &self.pk, &self.pkh, &self.target,
        );
        if let Some(d) = &self.delegate {
            s.push(' ');
            s.push_str(d);
        }
        Ok(s)
    }

    fn serialize_for_verification(&self) -> Result<Vec<u8>> {
        Ok(encode_string(&self.serialize()?))
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
            .map(Self::from_str)
        {
            Some(Ok(t)) => Outcome::Success(t),
            _ => Outcome::Forward(()),
        }
    }
}

impl AuthorizationToken for TezosAuthorizationString {
    fn resource(&self) -> &ResourceId {
        &self.target
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
            "Tezos Signed Message: {} {} {} {} {}",
            &self.domain, &self.timestamp, &self.pk, &self.pkh, &self.target,
        )?;
        if let Some(d) = &self.delegate {
            write!(f, " {}", &d)?;
        }
        write!(f, " {}", self.sig)
    }
}

#[rocket::async_trait]
impl AuthorizationPolicy<TezosAuthorizationString> for Manifest {
    async fn authorize(&self, auth_token: &TezosAuthorizationString) -> Result<()> {
        let requester = DIDURL {
            did: format!("did:pkh:tz:{}", &auth_token.pkh),
            fragment: Some("TezosMethod2021".to_string()),
            ..Default::default()
        };

        if !self.invokers().contains(&requester) {
            Err(anyhow!("Requester not a controller of the orbit"))
        } else {
            auth_token.verify()
        }
    }
}

#[rocket::async_trait]
impl Invoke<TezosAuthorizationString> for Orbit {
    async fn invoke(&self, invocation: &TezosAuthorizationString) -> Result<AuthRef> {
        unimplemented!()
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
    let auth_str = "Tezos Signed Message: kepler.net 2021-01-14T15:16:04Z edpkuyzsMxWoYepkwfkDesgc1QjQA6ND9VvRHGbjEPrBfuXRgwT4rv tz1VsijPb6UCRnrKUecDv8s8cKZAgJtyKyGc kepler:did:example://my-orbit/s3/my/file/path.jpg#get edsigtzNQz7kEhJjPj92fTYSazxsMgDJX5DH7s1DagAmZZqGfvgybhgmgZRsjcpVh9f84DjzpoVwTeGDw8H6GZqW8PHR5zeRGeU";
    let _: TezosAuthorizationString = auth_str.parse().unwrap();
}

#[test]
#[should_panic]
async fn simple_verify_fail() {
    let auth_str = "Tezos Signed Message: kepler.net 2021-01-14T15:16:04Z edpkuyzsMxWoYepkwfkDesgc1QjQA6ND9VvRHGbjEPrBfuXRgwT4rv tz1VsijPb6UCRnrKUecDv8s8cKZAgJtyKyGc kepler:did:example://my-orbit0/s3/my/file/path.jpg#get edsigtzNQz7kEhJjPj92fTYSazxsMgDJX5DH7s1DagAmZZqGfvgybhgmgZRsjcpVh9f84DjzpoVwTeGDw8H6GZqW8PHR5zeRGeU";
    let tza: TezosAuthorizationString = auth_str.parse().unwrap();
    tza.verify().unwrap();
}

#[test]
async fn simple_verify_succeed() {
    let auth_str = "Tezos Signed Message: kepler.net 2021-01-14T15:16:04Z edpkuyzsMxWoYepkwfkDesgc1QjQA6ND9VvRHGbjEPrBfuXRgwT4rv tz1VsijPb6UCRnrKUecDv8s8cKZAgJtyKyGc kepler:did:example://my-orbit/s3/my/file/path.jpg#get edsigtzNQz7kEhJjPj92fTYSazxsMgDJX5DH7s1DagAmZZqGfvgybhgmgZRsjcpVh9f84DjzpoVwTeGDw8H6GZqW8PHR5zeRGeU";
    let tza: TezosAuthorizationString = auth_str.parse().unwrap();
    tza.verify().unwrap();
}

#[test]
async fn round_trip() {
    use didkit::DID_METHODS;
    use ssi::{
        did::Source,
        jwk::{Algorithm, Params, JWK},
    };

    let ts = "2021-01-14T15:16:04Z";
    let dummy_resource = "kepler:did:example://my-orbit/s3/my/file/path.jpg#get";
    let j = JWK::generate_ed25519().unwrap();
    let did = DID_METHODS
        .generate(&Source::KeyAndPattern(&j, "tz"))
        .unwrap();
    let pkh = did.split(':').last().unwrap();
    let pk: String = match &j.params {
        Params::OKP(p) => bs58::encode(
            [13, 15, 37, 217]
                .iter()
                .chain(&p.public_key.0)
                .copied()
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
        target: dummy_resource.parse().unwrap(),
        delegate: None,
    };
    let message = tz_unsigned
        .serialize_for_verification()
        .expect("failed to serialize authz message");
    let sig_bytes = ssi::jws::sign_bytes(Algorithm::EdBlake2b, &message, &j).unwrap();
    let sig = bs58::encode(
        [9, 245, 205, 134, 18]
            .iter()
            .chain(&sig_bytes)
            .copied()
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
