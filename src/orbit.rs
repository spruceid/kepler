use crate::{
    auth::{cid_serde, Action, AuthorizationPolicy, AuthorizationToken},
    cas::ContentAddressedStorage,
    codec::SupportedCodecs,
    tz::{TezosAuthorizationString, TezosBasicAuthorization},
    zcap::{ZCAPAuthorization, ZCAPTokens},
};
use anyhow::{anyhow, Result};
use ipfs_embed::{Config, Ipfs, PeerId};
use libipld::{
    cid::{
        multibase::Base,
        multihash::{Code, MultihashDigest},
        Cid,
    },
    store::DefaultParams,
};
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
    tokio::fs,
};

use cached::proc_macro::cached;
use serde::{Deserialize, Serialize};
use ssi::did::DIDURL;
use std::{collections::HashMap, convert::TryFrom, path::PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct OrbitMetadata {
    // NOTE This will always serialize in b58check
    #[serde(with = "cid_serde")]
    pub id: Cid,
    pub controllers: Vec<DIDURL>,
    pub read_delegators: Vec<DIDURL>,
    pub write_delegators: Vec<DIDURL>,
    // TODO placeholder type
    pub revocations: Vec<String>,
    #[serde(default)]
    pub auth: AuthTypes,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(untagged, rename_all = "UPPERCASE")]
pub enum AuthTypes {
    Tezos,
    ZCAP,
}

impl Default for AuthTypes {
    fn default() -> Self {
        Self::Tezos
    }
}

impl From<&AuthMethods> for AuthTypes {
    fn from(m: &AuthMethods) -> Self {
        match m {
            AuthMethods::Tezos(_) => Self::Tezos,
            AuthMethods::ZCAP(_) => Self::ZCAP,
        }
    }
}

#[derive(Clone)]
pub enum AuthMethods {
    Tezos(TezosBasicAuthorization),
    ZCAP(ZCAPAuthorization),
}

#[derive(Clone)]
pub enum AuthTokens {
    Tezos(TezosAuthorizationString),
    ZCAP(ZCAPTokens),
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthTokens {
    type Error = anyhow::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        if let Outcome::Success(tz) = TezosAuthorizationString::from_request(request).await {
            Outcome::Success(Self::Tezos(tz))
        } else if let Outcome::Success(zcap) = ZCAPTokens::from_request(request).await {
            Outcome::Success(Self::ZCAP(zcap))
        } else {
            Outcome::Failure((
                Status::Unauthorized,
                anyhow!("No valid authorization headers"),
            ))
        }
    }
}

impl AuthorizationToken for AuthTokens {
    fn action(&self) -> Action {
        match self {
            Self::Tezos(token) => token.action(),
            Self::ZCAP(token) => token.action(),
        }
    }
    fn target_orbit(&self) -> &Cid {
        match self {
            Self::Tezos(token) => token.target_orbit(),
            Self::ZCAP(token) => token.target_orbit(),
        }
    }
}

impl AuthMethods {
    pub async fn authorize(&self, auth_token: AuthTokens) -> Result<()> {
        match self {
            Self::Tezos(method) => match auth_token {
                AuthTokens::Tezos(token) => method.authorize(&token).await,
                _ => return Err(anyhow!("Bad token")),
            },
            Self::ZCAP(method) => match auth_token {
                AuthTokens::ZCAP(token) => method.authorize(&token).await,
                _ => return Err(anyhow!("Bad token")),
            },
        }
    }
}

#[derive(Clone)]
pub struct Orbit {
    ipfs: Ipfs<DefaultParams>,
    metadata: OrbitMetadata,
    policy: AuthMethods,
}

// Using Option to distinguish when the orbit already exists from a hard error
pub async fn create_orbit(
    oid: Cid,
    path: PathBuf,
    controllers: Vec<DIDURL>,
    auth: &[u8],
    auth_type: AuthTypes,
) -> Result<Option<Orbit>> {
    let dir = path.join(oid.to_string_of_base(Base::Base58Btc)?);

    // fails if DIR exists, this is Create, not Open
    if dir.exists() {
        return Ok(None);
    }
    fs::create_dir(&dir)
        .await
        .map_err(|e| anyhow!("Couldn't create dir: {}", e))?;

    // create default and write
    let md = OrbitMetadata {
        id: oid.clone(),
        controllers: controllers,
        read_delegators: vec![],
        write_delegators: vec![],
        revocations: vec![],
        auth: auth_type,
    };
    fs::write(dir.join("metadata"), serde_json::to_vec_pretty(&md)?).await?;
    fs::write(dir.join("access_log"), auth).await?;

    Ok(Some(load_orbit(oid, path).await.map(|o| {
        o.ok_or_else(|| anyhow!("Couldn't find newly created orbit"))
    })??))
}

pub async fn load_orbit(oid: Cid, path: PathBuf) -> Result<Option<Orbit>> {
    let dir = path.join(oid.to_string_of_base(Base::Base58Btc)?);
    if !dir.exists() {
        return Ok(None);
    }
    load_orbit_(oid, dir).await.map(|o| Some(o))
}

// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
// 1min timeout to evict orbits that might have been deleted
#[cached(size = 100, time = 60, result = true)]
async fn load_orbit_(oid: Cid, dir: PathBuf) -> Result<Orbit> {
    let mut cfg = Config::new(Some(dir.join("block_store")), 0);
    cfg.network.mdns = None;
    cfg.network.gossipsub = None;
    cfg.network.broadcast = None;
    cfg.network.bitswap = None;

    let md: OrbitMetadata = serde_json::from_slice(&fs::read(dir.join("metadata")).await?)?;

    // TODO enable dht once orbits are defined
    cfg.network.kad = None;
    let ipfs = Ipfs::<DefaultParams>::new(cfg).await?;
    let controllers = md.controllers.clone();

    Ok(Orbit {
        ipfs,
        policy: match &md.auth {
            AuthTypes::Tezos => AuthMethods::Tezos(TezosBasicAuthorization { controllers }),
            AuthTypes::ZCAP => AuthMethods::ZCAP(controllers),
        },
        metadata: md,
    })
}

pub fn get_params<'a>(matrix_params: &'a str) -> HashMap<&'a str, &'a str> {
    matrix_params
        .split(";")
        .fold(HashMap::new(), |mut acc, pair_str| {
            let mut ps = pair_str.split("=");
            match (ps.next(), ps.next(), ps.next()) {
                (Some(key), Some(value), None) => acc.insert(key, value),
                _ => None,
            };
            acc
        })
}

pub fn verify_oid<'a>(oid: &Cid, uri_str: &'a str) -> Result<(&'a str, HashMap<&'a str, &'a str>)> {
    // try to parse as a URI with matrix params
    if &Code::try_from(oid.hash().code())?.digest(uri_str.as_bytes()) == oid.hash()
        && oid.codec() == 0x55
    {
        let first_sc = uri_str.find(";").unwrap_or(uri_str.len());
        Ok((
            // method name
            uri_str
                .get(..first_sc)
                .ok_or(anyhow!("Missing Orbit Method"))?,
            // matrix parameters
            get_params(uri_str.get(first_sc..).unwrap_or("")),
        ))
    } else {
        Err(anyhow!("Failed to verify Orbit ID"))
    }
}

#[rocket::async_trait]
impl ContentAddressedStorage for Orbit {
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

impl Orbit {
    pub fn id(&self) -> &Cid {
        &self.metadata.id
    }

    pub fn hosts(&self) -> Vec<PeerId> {
        vec![self.ipfs.local_peer_id()]
    }

    pub fn admins(&self) -> &[DIDURL] {
        &self.metadata.controllers
    }

    pub fn auth(&self) -> &AuthMethods {
        &self.policy
    }

    pub fn make_uri(&self, cid: &Cid) -> Result<String> {
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
    let (method, params) = verify_oid(&oid, &uri).unwrap();
    assert_eq!(method, "tz");
    assert_eq!(params.get("address"), Some(&pkh));
    assert_eq!(params.get("domain"), Some(&domain));
    assert_eq!(params.get("index"), Some(&"0"));
}
