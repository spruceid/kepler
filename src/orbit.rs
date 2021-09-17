use crate::{
    auth::{cid_serde, Action, AuthorizationPolicy, AuthorizationToken},
    cas::ContentAddressedStorage,
    codec::SupportedCodecs,
    ipfs_embed::{
        db::{open_store, StorageConfig, StorageService},
        open_orbit_ipfs,
    },
    tz::{TezosAuthorizationString, TezosBasicAuthorization},
    tz_orbit::params_to_tz_orbit,
    zcap::{ZCAPAuthorization, ZCAPTokens},
};
use anyhow::{anyhow, Result};
use libipld::{
    cid::{
        multibase::Base,
        multihash::{Code, MultihashDigest},
        Cid,
    },
    store::DefaultParams,
};
use libp2p::{Multiaddr, PeerId};
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
    tokio::fs,
};

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

#[derive(Serialize, Deserialize, Clone)]
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
    #[serde(default)]
    pub auth: AuthTypes,
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
        match (self, auth_token) {
            (Self::Tezos(method), AuthTokens::Tezos(token)) => method.authorize(&token).await,
            (Self::ZCAP(method), AuthTokens::ZCAP(token)) => method.authorize(&token).await,
            _ => return Err(anyhow!("Bad token")),
        }
    }
}

#[derive(Clone)]
pub struct Orbit {
    storage: StorageService<DefaultParams>,
    oid: Cid,
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
    uri: &str,
    tzkt_api: &str,
    relay_addr: Multiaddr,
) -> Result<Option<Orbit>> {
    let dir = path.join(oid.to_string_of_base(Base::Base58Btc)?);

    // fails if DIR exists, this is Create, not Open
    if dir.exists() {
        return Ok(None);
    }
    fs::create_dir(&dir)
        .await
        .map_err(|e| anyhow!("Couldn't create dir: {}", e))?;

    let (method, params) = verify_oid(&oid, uri)?;

    let md = match method {
        "tz" => params_to_tz_orbit(oid, &params, tzkt_api).await?,
        _ => OrbitMetadata {
            id: oid.clone(),
            controllers,
            read_delegators: vec![],
            write_delegators: vec![],
            revocations: vec![],
            auth: auth_type,
            hosts: Map::default(),
        },
    };

    fs::write(dir.join("metadata"), serde_json::to_vec_pretty(&md)?).await?;
    fs::write(dir.join("access_log"), auth).await?;

    let orbit = load_orbit(oid, path)
        .await
        .map(|o| o.ok_or_else(|| anyhow!("Couldn't find newly created orbit")))??;

    open_orbit_ipfs(oid, dir, relay_addr).await?;

    Ok(Some(orbit))
}

pub async fn load_orbit(oid: Cid, path: PathBuf) -> Result<Option<Orbit>> {
    let dir = path.join(oid.to_string_of_base(Base::Base58Btc)?);
    if !dir.exists() {
        return Ok(None);
    }
    load_orbit_(oid, dir).await.map(|o| Some(o))
}

// TODO oid and dir are redundant
//
// Only used for reads
//
// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
// 1min timeout to evict orbits that might have been deleted
// #[cached(size = 100, time = 60, result = true)]
async fn load_orbit_(oid: Cid, dir: PathBuf) -> Result<Orbit> {
    // let mut cfg = db::Config::new(&dir.join("block_store"), generate_keypair());
    let sweep_interval = std::time::Duration::from_millis(10000);
    let storage_config = StorageConfig::new(
        oid.to_string(),
        dir.join("block_store").join("blocks"),
        sweep_interval,
    );
    // let network = NetworkConfig::new(path.join("streams"), keypair);
    // cfg.network.node_key = key_pair.0;

    let md: OrbitMetadata = serde_json::from_slice(&fs::read(dir.join("metadata")).await?)?;
    let controllers = md.controllers.clone();

    // let ipfs = Ipfs::<DefaultParams>::new(cfg).await?;
    let storage = open_store(oid, dir)?;
    // let bitswap = BitswapStorage {
    //     oid,
    //     dir,
    // };
    // let network = NetworkService::new(config.network, bitswap, executor).await?;

    // if let Some(addrs) = md.hosts.get(&PID(ipfs.local_peer_id())) {
    //     for addr in addrs {
    //         ipfs.listen_on(addr.clone())?.next().await;
    //     }
    // }

    // for (id, addrs) in md.hosts.iter() {
    //     if id.0 != ipfs.local_peer_id() {
    //         for addr in addrs {
    //             ipfs.add_address(&id.0, addr.clone())
    //         }
    //     }
    // }

    Ok(Orbit {
        storage,
        oid,
        policy: match &md.auth {
            AuthTypes::Tezos => AuthMethods::Tezos(TezosBasicAuthorization { controllers }),
            AuthTypes::ZCAP => AuthMethods::ZCAP(controllers),
        },
        metadata: md,
    })
}

pub fn get_params<'a>(matrix_params: &'a str) -> Map<&'a str, &'a str> {
    matrix_params
        .split(";")
        .fold(Map::new(), |mut acc, pair_str| {
            let mut ps = pair_str.split("=");
            match (ps.next(), ps.next(), ps.next()) {
                (Some(key), Some(value), None) => acc.insert(key, value),
                _ => None,
            };
            acc
        })
}

pub fn verify_oid<'a>(oid: &Cid, uri_str: &'a str) -> Result<(&'a str, Map<&'a str, &'a str>)> {
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
        self.storage.put(content, codec).await
    }
    async fn get(
        &self,
        address: &Cid,
    ) -> Result<Option<Vec<u8>>, <Self as ContentAddressedStorage>::Error> {
        ContentAddressedStorage::get(&self.storage, address).await
    }
    async fn delete(&self, address: &Cid) -> Result<(), <Self as ContentAddressedStorage>::Error> {
        self.storage.delete(address).await
    }
    async fn list(&self) -> Result<Vec<Cid>, <Self as ContentAddressedStorage>::Error> {
        self.storage.list().await
    }
}

impl Orbit {
    pub fn id(&self) -> &Cid {
        &self.metadata.id
    }

    pub fn hosts(&self) -> Vec<PeerId> {
        // vec![self.storage.local_peer_id()]
        vec![]
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

// #[test]
// async fn oid_verification() {
//     let oid: Cid = "zCT5htkeBtA6Qu5YF4vPkQcfeqy3pY4m8zxGdUKUiPgtPEbY3rHy"
//         .parse()
//         .unwrap();
//     let pkh = "tz1YSb7gXhgBw46nSXthhoSzhJdbQf9h92Gy";
//     let domain = "kepler.tzprofiles.com";
//     let index = 0;
//     let uri = format!("tz;address={};domain={};index={}", pkh, domain, index);
//     let (method, params) = verify_oid(&oid, &uri).unwrap();
//     assert_eq!(method, "tz");
//     assert_eq!(params.get("address"), Some(&pkh));
//     assert_eq!(params.get("domain"), Some(&domain));
//     assert_eq!(params.get("index"), Some(&"0"));
// }

// #[test]
// async fn oid_verification() {
//     let oid: Cid = "zCT5htkeBtA6Qu5YF4vPkQcfeqy3pY4m8zxGdUKUiPgtPEbY3rHy"
//         .parse()
//         .unwrap();
//     let pkh = "tz1YSb7gXhgBw46nSXthhoSzhJdbQf9h92Gy";
//     let domain = "kepler.tzprofiles.com";
//     let index = 0;
//     let uri = format!("tz;address={};domain={};index={}", pkh, domain, index);
//     verify_oid(&oid, pkh, &uri).unwrap();
// }
