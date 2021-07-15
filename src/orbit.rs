use crate::{
    auth::{Action, AuthorizationPolicy, AuthorizationToken},
    cas::ContentAddressedStorage,
    codec::SupportedCodecs,
    tz::{TezosAuthorizationString, TezosBasicAuthorization},
    tz_orbit::params_to_tz_orbit,
};
use anyhow::{anyhow, Result};
use ipfs_embed::{Config, Ipfs, Keypair, Multiaddr, PeerId};
use libipld::{
    cid::{
        multibase::Base,
        multihash::{Code, MultihashDigest},
        Cid,
    },
    store::DefaultParams,
};
use rocket::tokio::fs;

use cached::proc_macro::cached;
use serde::{Deserialize, Serialize};
use ssi::did::DIDURL;
use std::{
    collections::HashMap as Map,
    convert::TryFrom,
    hash::{Hash, Hasher},
    ops::Deref,
    path::PathBuf,
    str::FromStr,
};

#[derive(Serialize, Deserialize, Debug)]
pub struct OrbitMetadata {
    // NOTE This will always serialize in b58check
    #[serde(with = "cid_serde")]
    pub id: Cid,
    pub controllers: Vec<DIDURL>,
    pub read_delegators: Vec<DIDURL>,
    pub write_delegators: Vec<DIDURL>,
    #[serde(default)]
    pub hosts: Map<PID, Vec<Multiaddr>>,
    // TODO placeholder type
    pub revocations: Vec<String>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Hash, Debug)]
#[serde(try_from = "&str", into = "String")]
pub struct PID(pub PeerId);

impl Deref for PID {
    type Target = PeerId;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TryFrom<&str> for PID {
    type Error = <PeerId as FromStr>::Err;
    fn try_from(v: &str) -> Result<Self, Self::Error> {
        Ok(Self(PeerId::from_str(v)?))
    }
}

impl From<PID> for String {
    fn from(pid: PID) -> Self {
        pid.to_base58()
    }
}

mod cid_serde {
    use libipld::cid::{multibase::Base, Cid};
    use serde::{de::Error as SError, ser::Error as DError, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(cid: &Cid, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ser.serialize_str(
            &cid.to_string_of_base(Base::Base58Btc)
                .map_err(S::Error::custom)?,
        )
    }

    pub fn deserialize<'de, D>(deser: D) -> Result<Cid, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &str = Deserialize::deserialize(deser)?;
        s.parse().map_err(D::Error::custom)
    }
}

// TODO I think this will need to go, using a trait for a core object creates
// too much monomorphisation which shouldn't be too much of a problem for the
// binary size but I'm not sure how Rocket handles it.
// Or maybe implement the traits separately.
// Ultimately I think we'll need to use an enum, it's not as fancy but because
// we only have simple relationships (e.g. for the Tezos policy you need a
// Tezos token) it will make the routing simpler and easier to use.
#[rocket::async_trait]
pub trait Orbit: ContentAddressedStorage {
    // + AuthorizationPolicy {
    fn id(&self) -> &Cid;

    fn hosts(&self) -> Vec<PeerId>;

    fn admins(&self) -> &[&DIDURL];

    fn auth(&self) -> &AuthMethods;

    fn make_uri(&self, cid: &Cid) -> Result<String>;

    // async fn update(
    //     &self,
    //     update: Self::UpdateMessage,
    // ) -> Result<(), <Self as ContentAddressedStorage>::Error>;
}

#[derive(Clone)]
pub enum AuthMethods {
    Tezos(TezosBasicAuthorization),
}

#[derive(Clone)]
pub enum AuthTokens {
    Tezos(TezosAuthorizationString),
}

impl AuthorizationToken for AuthTokens {
    fn extract(auth_data: &str) -> Result<Self> {
        Err(anyhow!("todo"))
    }

    fn action(&self) -> &Action {
        match self {
            Self::Tezos(token) => token.action(),
        }
    }
}

impl AuthMethods {
    pub async fn authorize(&self, auth_token: AuthTokens) -> Result<()> {
        match self {
            Self::Tezos(method) => match auth_token {
                AuthTokens::Tezos(token) => method.authorize(&token).await,
            },
        }
    }
}

#[derive(Clone)]
pub struct SimpleOrbit {
    ipfs: Ipfs<DefaultParams>,
    oid: Cid,
    policy: AuthMethods,
}

// Using Option to distinguish when the orbit already exists from a hard error
pub async fn create_orbit(
    oid: Cid,
    path: PathBuf,
    auth: &[u8],
    uri: &str,
    key_pair: &Keypair,
) -> Result<Option<SimpleOrbit>> {
    let dir = path.join(oid.to_string_of_base(Base::Base58Btc)?);

    // fails if DIR exists, this is Create, not Open
    if dir.exists() {
        return Ok(None);
    }
    fs::create_dir(&dir)
        .await
        .map_err(|e| anyhow!("Couldn't create dir: {}", e))?;

    let (method, params) = get_oid_matrix_params(uri)?;

    let md = match method {
        "tz" => params_to_tz_orbit(oid, &params.collect::<Vec<(&str, &str)>>()).await?,
        _ => return Err(anyhow!("Unsupported method type: {}", method)),
    };

    fs::write(dir.join("metadata"), serde_json::to_vec_pretty(&md)?).await?;
    fs::write(dir.join("access_log"), auth).await?;

    Ok(Some(load_orbit(oid, path, key_pair).await.map(|o| {
        o.ok_or_else(|| anyhow!("Couldn't find newly created orbit"))
    })??))
}

pub async fn load_orbit(
    oid: Cid,
    path: PathBuf,
    key_pair: &Keypair,
) -> Result<Option<SimpleOrbit>> {
    let dir = path.join(oid.to_string_of_base(Base::Base58Btc)?);
    if !dir.exists() {
        return Ok(None);
    }
    load_orbit_(oid, dir, key_pair.into())
        .await
        .map(|o| Some(o))
}

struct KP(pub Keypair);

impl From<&Keypair> for KP {
    fn from(kp: &Keypair) -> Self {
        KP(Keypair::from_bytes(&kp.to_bytes()).unwrap())
    }
}

impl Clone for KP {
    fn clone(&self) -> Self {
        KP(Keypair::from_bytes(&self.0.to_bytes()).unwrap())
    }
}

impl PartialEq for KP {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bytes() == other.0.to_bytes()
    }
}

impl Eq for KP {}

impl Hash for KP {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bytes().hash(state);
    }
}

// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
// 1min timeout to evict orbits that might have been deleted
#[cached(size = 100, time = 60, result = true)]
async fn load_orbit_(oid: Cid, dir: PathBuf, key_pair: KP) -> Result<SimpleOrbit> {
    let mut cfg = Config::new(Some(dir.join("block_store")), 0);
    cfg.network.mdns = None;
    cfg.network.gossipsub = None;
    cfg.network.broadcast = None;
    cfg.network.bitswap = None;
    cfg.network.node_key = key_pair.0;

    let md: OrbitMetadata = serde_json::from_slice(&fs::read(dir.join("metadata")).await?)?;

    // TODO enable dht once orbits are defined
    cfg.network.kad = None;
    let ipfs = Ipfs::<DefaultParams>::new(cfg).await?;

    Ok(SimpleOrbit {
        ipfs,
        oid,
        policy: AuthMethods::Tezos(TezosBasicAuthorization {
            controllers: md.controllers,
        }),
    })
}

pub fn get_oid_matrix_params<'a>(
    uri: &'a str,
) -> Result<(&'a str, impl Iterator<Item = (&'a str, &'a str)>)> {
    let mut parts = uri.split(';');
    let method = parts.next().ok_or(anyhow!("No URI"))?;

    Ok((
        method,
        parts.filter_map(|part| {
            let mut kvs = part.split("=");
            if let (Some(k), Some(v), None) = (kvs.next(), kvs.next(), kvs.next()) {
                Some((k, v))
            } else {
                None
            }
        }),
    ))
}

pub fn verify_oid(oid: &Cid, uri_str: &str) -> Result<()> {
    if &Code::try_from(oid.hash().code())?.digest(uri_str.as_bytes()) == oid.hash()
        && oid.codec() == 0x55
    {
        Ok(())
    } else {
        Err(anyhow!("Failed to verify Orbit ID"))
    }
}

#[rocket::async_trait]
impl ContentAddressedStorage for SimpleOrbit {
    type Error = anyhow::Error;
    async fn put(
        &self,
        content: &[u8],
        codec: SupportedCodecs,
    ) -> Result<Cid, <Self as ContentAddressedStorage>::Error> {
        self.ipfs.put(content, codec).await
    }
    async fn get(
        &self,
        address: &Cid,
    ) -> Result<Option<Vec<u8>>, <Self as ContentAddressedStorage>::Error> {
        ContentAddressedStorage::get(&self.ipfs, address).await
    }
    async fn delete(&self, address: &Cid) -> Result<(), <Self as ContentAddressedStorage>::Error> {
        self.ipfs.delete(address).await
    }
    async fn list(&self) -> Result<Vec<Cid>, <Self as ContentAddressedStorage>::Error> {
        self.ipfs.list().await
    }
}

#[rocket::async_trait]
impl Orbit for SimpleOrbit {
    fn id(&self) -> &Cid {
        &self.oid
    }

    fn hosts(&self) -> Vec<PeerId> {
        vec![self.ipfs.local_peer_id()]
    }

    fn admins(&self) -> &[&DIDURL] {
        todo!()
    }

    fn auth(&self) -> &AuthMethods {
        &self.policy
    }

    fn make_uri(&self, cid: &Cid) -> Result<String> {
        Ok(format!(
            "kepler://{}/{}",
            self.id().to_string_of_base(Base::Base58Btc)?,
            cid.to_string_of_base(Base::Base58Btc)?
        ))
    }

    // async fn update(&self, _update: Self::UpdateMessage) -> Result<(), <Self as Orbit>::Error> {
    //     todo!()
    // }
}

#[test]
async fn oid_verification() {
    let oid: Cid = "zCT5htkeBtA6Qu5YF4vPkQcfeqy3pY4m8zxGdUKUiPgtPEbY3rHy"
        .parse()
        .unwrap();
    let pkh = "tz1YSb7gXhgBw46nSXthhoSzhJdbQf9h92Gy";
    let domain = "kepler.tzprofiles.com";
    let index = 0;
    let uri = format!("tz;address={};domain={};index={}", pkh, domain, index);
    verify_oid(&oid, pkh, &uri).unwrap();
}
