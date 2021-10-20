use crate::{
    auth::{Action, AuthorizationPolicy, AuthorizationToken},
    cas::ContentAddressedStorage,
    codec::SupportedCodecs,
    config::ExternalApis,
    tz::TezosAuthorizationString,
    tz_orbit::params_to_tz_orbit,
    zcap::ZCAPTokens,
};
use anyhow::{anyhow, Result};
use ipfs_embed::{
    generate_keypair, multiaddr::multiaddr, Config, Ipfs, Keypair, Multiaddr, PeerId,
};
use libipld::{
    cid::{
        multibase::Base,
        multihash::{Code, MultihashDigest},
        Cid,
    },
    store::DefaultParams,
};
use rocket::{
    futures::StreamExt,
    http::Status,
    request::{FromRequest, Outcome, Request},
    tokio::{fs, task::JoinHandle},
};

use cached::proc_macro::cached;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use ssi::did::DIDURL;
use std::{
    collections::HashMap as Map,
    convert::TryFrom,
    ops::Deref,
    path::PathBuf,
    sync::{Arc, RwLock},
};

#[serde_as]
#[derive(Serialize, Deserialize, Clone)]
pub struct OrbitMetadata {
    // NOTE This will always serialize in b58check
    #[serde_as(as = "DisplayFromStr")]
    pub id: Cid,
    pub controllers: Vec<DIDURL>,
    pub read_delegators: Vec<DIDURL>,
    pub write_delegators: Vec<DIDURL>,
    #[serde(default)]
    #[serde_as(as = "Map<DisplayFromStr, _>")]
    pub hosts: Map<PeerId, Vec<Multiaddr>>,
    // TODO placeholder type
    pub revocations: Vec<String>,
}

impl OrbitMetadata {
    pub fn hosts<'a>(&'a self) -> impl Iterator<Item = &'a PeerId> {
        self.hosts.keys()
    }
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
#[rocket::async_trait]
impl AuthorizationPolicy<AuthTokens> for Orbit {
    async fn authorize(&self, auth_token: &AuthTokens) -> Result<()> {
        match auth_token {
            AuthTokens::Tezos(token) => self.metadata.authorize(token).await,
            AuthTokens::ZCAP(token) => self.metadata.authorize(token).await,
        }
    }
}

struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> AbortOnDrop<T> {
    pub fn new(h: JoinHandle<T>) -> Self {
        Self(h)
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T> Deref for AbortOnDrop<T> {
    type Target = JoinHandle<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct Orbit {
    task: Arc<AbortOnDrop<()>>,
    pub service: Service,
    metadata: OrbitMetadata,
}

// Using Option to distinguish when the orbit already exists from a hard error
pub async fn create_orbit(
    oid: Cid,
    path: PathBuf,
    controllers: Vec<DIDURL>,
    auth: &[u8],
    uri: &str,
    chains: &ExternalApis,
    relay: (PeerId, Multiaddr),
    keys_lock: &RwLock<Map<PeerId, Keypair>>,
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

    let md = match (method.as_str(), &chains) {
        (
            "tz",
            ExternalApis {
                tzkt: Some(api), ..
            },
        ) => params_to_tz_orbit(oid, &params, &api).await?,
        _ => OrbitMetadata {
            id: oid.clone(),
            controllers: controllers,
            read_delegators: vec![],
            write_delegators: vec![],
            revocations: vec![],
            hosts: params
                .get("hosts")
                .map(|hs| parse_hosts_str(hs))
                .unwrap_or(Ok(Default::default()))?,
        },
    };

    let kp = {
        let mut keys = keys_lock.write().map_err(|e| anyhow!(e.to_string()))?;
        md.hosts()
            .find_map(|h| keys.remove(h))
            .unwrap_or(generate_keypair())
    };

    fs::write(dir.join("metadata"), serde_json::to_vec_pretty(&md)?).await?;
    fs::write(dir.join("access_log"), auth).await?;
    fs::write(dir.join("kp"), kp.to_bytes()).await?;

    Ok(Some(load_orbit(oid, path, relay).await.map(|o| {
        o.ok_or_else(|| anyhow!("Couldn't find newly created orbit"))
    })??))
}

pub async fn load_orbit(
    oid: Cid,
    path: PathBuf,
    relay: (PeerId, Multiaddr),
) -> Result<Option<Orbit>> {
    let dir = path.join(oid.to_string_of_base(Base::Base58Btc)?);
    if !dir.exists() {
        return Ok(None);
    }
    load_orbit_(oid, dir, relay).await.map(|o| Some(o))
}

// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
// 1min timeout to evict orbits that might have been deleted
#[cached(size = 100, time = 60, result = true)]
async fn load_orbit_(_oid: Cid, dir: PathBuf, relay: (PeerId, Multiaddr)) -> Result<Orbit> {
    let kp = Keypair::from_bytes(&fs::read(dir.join("kp")).await?)?;
    let cfg = Config::new(&dir.join("block_store"), kp);

    let md: OrbitMetadata = serde_json::from_slice(&fs::read(dir.join("metadata")).await?)?;
    let md2 = md.clone();

    let ipfs = Ipfs::<DefaultParams>::new(cfg).await?;

    // listen for any relayed messages
    ipfs.listen_on(multiaddr!(P2pCircuit))?.next().await;
    // establish a connection to the relay
    ipfs.dial_address(&relay.0, relay.1);

    let task_ipfs = ipfs.clone();
    let controllers = md.controllers.clone();

    let task = Arc::new(AbortOnDrop::new(tokio::spawn(async move {
        let mut events = task_ipfs.swarm_events();
        loop {
            match events.next().await {
                Some(ipfs_embed::Event::Discovered(p)) => {
                    tracing::debug!("dialing peer {}", p);
                    task_ipfs.dial(&p);
                }
                None => return,
                _ => continue,
            }
        }
    })));

    let id = md.id.to_string_of_base(Base::Base58Btc)?;
    let db = sled::open(dir.join(&id).with_extension("ks3db"))?;

    let service_store = Store::new(id, ipfs, db)?;
    let service = Service::start(service_store)?;

    Ok(Orbit {
        service,
        task,
        metadata: md,
    })
}

pub fn parse_hosts_str(s: &str) -> Result<Map<PeerId, Vec<Multiaddr>>> {
    s.split("|")
        .map(|hs| {
            hs.split_once(":")
                .ok_or(anyhow!("missing host:addrs map"))
                .and_then(|(id, s)| {
                    Ok((
                        id.parse()?,
                        s.split(",")
                            .map(|a| Ok(a.parse()?))
                            .collect::<Result<Vec<Multiaddr>>>()?,
                    ))
                })
        })
        .collect()
}

pub fn get_params(matrix_params: &str) -> Map<String, String> {
    matrix_params
        .split(";")
        .fold(Map::new(), |mut acc, pair_str| {
            match pair_str.split_once("=") {
                Some((key, value)) => acc.insert(key.into(), value.into()),
                _ => None,
            };
            acc
        })
}

pub fn verify_oid(oid: &Cid, uri_str: &str) -> Result<(String, Map<String, String>)> {
    // try to parse as a URI with matrix params
    if &Code::try_from(oid.hash().code())?.digest(uri_str.as_bytes()) == oid.hash()
        && oid.codec() == 0x55
    {
        let decoded = urlencoding::decode(uri_str)?;
        let first_sc = decoded.find(";").unwrap_or(uri_str.len());
        Ok((
            // method name
            decoded
                .get(..first_sc)
                .ok_or(anyhow!("Missing Orbit Method"))?
                .into(),
            // matrix parameters
            get_params(decoded.get(first_sc..).unwrap_or("")),
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
        self.service.ipfs.put(content, codec).await
    }
    async fn get(
        &self,
        address: &Cid,
    ) -> Result<Option<Vec<u8>>, <Self as ContentAddressedStorage>::Error> {
        ContentAddressedStorage::get(&self.service.ipfs, address).await
    }
    async fn delete(&self, address: &Cid) -> Result<(), <Self as ContentAddressedStorage>::Error> {
        self.service.ipfs.delete(address).await
    }
    async fn list(&self) -> Result<Vec<Cid>, <Self as ContentAddressedStorage>::Error> {
        self.service.ipfs.list().await
    }
}

impl Orbit {
    pub fn id(&self) -> &Cid {
        &self.metadata.id
    }

    pub fn hosts<'a>(&'a self) -> impl Iterator<Item = &'a PeerId> {
        self.metadata.hosts()
    }

    pub fn controllers(&self) -> &[DIDURL] {
        &self.metadata.controllers
    }

    pub fn read_delegators(&self) -> &[DIDURL] {
        &self.metadata.read_delegators
    }

    pub fn write_delegators(&self) -> &[DIDURL] {
        &self.metadata.write_delegators
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
    assert_eq!(params.get("address").unwrap(), pkh);
    assert_eq!(params.get("domain").unwrap(), domain);
    assert_eq!(params.get("index").unwrap(), "0");
}
