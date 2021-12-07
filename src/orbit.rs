use crate::{
    auth::{Action, AuthorizationPolicy, AuthorizationToken},
    cas::ContentAddressedStorage,
    codec::SupportedCodecs,
    config::ExternalApis,
    ipfs::Ipfs,
    s3::{Service, Store},
    siwe::SIWETokens,
    tz::TezosAuthorizationString,
    tz_orbit::params_to_tz_orbit,
    zcap::ZCAPTokens,
};
use anyhow::{anyhow, Result};
use ipfs_embed::{generate_keypair, multiaddr::multiaddr, Config, Keypair, Multiaddr, PeerId};
use libipld::cid::{
    multibase::Base,
    multihash::{Code, MultihashDigest},
    Cid,
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
#[derive(Serialize, Deserialize, Clone, Debug)]
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
    pub fn id(&self) -> &Cid {
        &self.id
    }

    pub fn hosts<'a>(&'a self) -> impl Iterator<Item = &'a PeerId> {
        self.hosts.keys()
    }

    pub fn controllers(&self) -> &[DIDURL] {
        &self.controllers
    }

    pub fn make_uri(&self, cid: &Cid) -> Result<String> {
        Ok(format!(
            "kepler://{}/{}",
            self.id().to_string_of_base(Base::Base58Btc)?,
            cid.to_string_of_base(Base::Base58Btc)?
        ))
    }
}

pub enum AuthTokens {
    Tezos(TezosAuthorizationString),
    ZCAP(ZCAPTokens),
    SIWE(SIWETokens),
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthTokens {
    type Error = anyhow::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let ats =
            if let Outcome::Success(tz) = TezosAuthorizationString::from_request(request).await {
                Self::Tezos(tz)
            } else if let Outcome::Success(zcap) = ZCAPTokens::from_request(request).await {
                Self::ZCAP(zcap)
            } else if let Outcome::Success(siwe) = SIWETokens::from_request(request).await {
                Self::SIWE(siwe)
            } else {
                return Outcome::Failure((
                    Status::Unauthorized,
                    anyhow!("No valid authorization headers"),
                ));
            };
        Outcome::Success(ats)
    }
}

impl AuthorizationToken for AuthTokens {
    fn action(&self) -> &Action {
        match self {
            Self::Tezos(token) => token.action(),
            Self::ZCAP(token) => token.action(),
            Self::SIWE(token) => token.action(),
        }
    }
    fn target_orbit(&self) -> &Cid {
        match self {
            Self::Tezos(token) => token.target_orbit(),
            Self::ZCAP(token) => token.target_orbit(),
            Self::SIWE(token) => token.target_orbit(),
        }
    }
}
#[rocket::async_trait]
impl AuthorizationPolicy<AuthTokens> for OrbitMetadata {
    async fn authorize(&self, auth_token: &AuthTokens) -> Result<()> {
        match auth_token {
            AuthTokens::Tezos(token) => self.authorize(token).await,
            AuthTokens::ZCAP(token) => self.authorize(token).await,
            AuthTokens::SIWE(token) => self.authorize(token).await,
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

fn get_params_vm(method: &str, params: &Map<String, String>) -> Option<DIDURL> {
    match method {
        "tz" => match (params.get("address"), params.get("contract")) {
            (_, Some(_contract)) => None,
            (Some(address), None) => Some(DIDURL {
                did: format!("did:pkh:tz:{}", &address),
                fragment: Some("TezosMethod2021".to_string()),
                ..Default::default()
            }),
            _ => None,
        },
        "did" => match (params.get("did"), params.get("vm")) {
            (Some(did), Some(vm_id)) => Some(DIDURL {
                did: did.into(),
                fragment: Some(vm_id.into()),
                ..Default::default()
            }),
            _ => None,
        },
        _ => None,
    }
}

pub async fn get_metadata(
    oid: &Cid,
    param_str: &str,
    chains: &ExternalApis,
) -> Result<OrbitMetadata> {
    let (method, params) = verify_oid(oid, param_str)?;
    Ok(match (method.as_str(), &chains) {
        ("tz", ExternalApis { tzkt, .. }) => params_to_tz_orbit(*oid, &params, &tzkt).await?,
        _ => OrbitMetadata {
            id: *oid,
            controllers: vec![get_params_vm(method.as_ref(), &params)
                .ok_or(anyhow!("Missing Implicit Controller Params"))?],
            read_delegators: vec![],
            write_delegators: vec![],
            revocations: vec![],
            hosts: params
                .get("hosts")
                .map(|hs| parse_hosts_str(hs))
                .unwrap_or(Ok(Default::default()))?,
        },
    })
}

// Using Option to distinguish when the orbit already exists from a hard error
pub async fn create_orbit(
    md: &OrbitMetadata,
    path: PathBuf,
    auth: &[u8],
    relay: (PeerId, Multiaddr),
    keys_lock: &RwLock<Map<PeerId, Keypair>>,
) -> Result<Option<Orbit>> {
    let dir = path.join(md.id.to_string_of_base(Base::Base58Btc)?);

    // fails if DIR exists, this is Create, not Open
    if dir.exists() {
        return Ok(None);
    }
    fs::create_dir(&dir)
        .await
        .map_err(|e| anyhow!("Couldn't create dir: {}", e))?;

    let kp = {
        let mut keys = keys_lock.write().map_err(|e| anyhow!(e.to_string()))?;
        md.hosts()
            .find_map(|h| keys.remove(h))
            .unwrap_or(generate_keypair())
    };

    fs::write(dir.join("metadata"), serde_json::to_vec_pretty(md)?).await?;
    fs::write(dir.join("access_log"), auth).await?;
    fs::write(dir.join("kp"), kp.to_bytes()).await?;

    Ok(Some(load_orbit(md.id, path, relay).await.map(|o| {
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
    load_orbit_(dir, relay).await.map(|o| Some(o))
}

// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
// 1min timeout to evict orbits that might have been deleted
#[cached(size = 100, time = 60, result = true, sync_writes = true)]
async fn load_orbit_(dir: PathBuf, relay: (PeerId, Multiaddr)) -> Result<Orbit> {
    let kp = Keypair::from_bytes(&fs::read(dir.join("kp")).await?)?;
    let mut cfg = Config::new(&dir.join("block_store"), kp);
    cfg.network.streams = None;

    let md: OrbitMetadata = serde_json::from_slice(&fs::read(dir.join("metadata")).await?)?;
    let id = md.id.to_string_of_base(Base::Base58Btc)?;
    tracing::debug!("loading orbit {}, {:?}", &id, &dir);

    let ipfs = Ipfs::new(cfg).await?;

    // listen for any relayed messages
    ipfs.listen_on(multiaddr!(P2pCircuit))?.next().await;
    // establish a connection to the relay
    ipfs.dial_address(&relay.0, relay.1);

    for (peer, addrs) in md.hosts.iter() {
        if peer != &ipfs.local_peer_id() {
            for addr in addrs.iter() {
                ipfs.dial_address(peer, addr.clone());
            }
        }
    }

    let task_ipfs = ipfs.clone();

    let db = sled::open(dir.join(&id).with_extension("ks3db"))?;

    let service_store = Store::new(id, ipfs, db)?;
    let service = Service::start(service_store)?;

    let st = service.store.clone();

    let task = Arc::new(AbortOnDrop::new(tokio::spawn(async move {
        let mut events = st.ipfs.swarm_events();
        loop {
            match events.next().await {
                Some(ipfs_embed::Event::Discovered(p)) => {
                    if task_ipfs.peers().contains(&p) {
                        tracing::debug!("dialing peer {}", p);
                        task_ipfs.dial(&p);
                        st.request_heads();
                    } else {
                        task_ipfs.ban(p)
                    };
                }
                None => return,
                _ => continue,
            }
        }
    })));

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

pub fn get_params(matrix_params: &str) -> Result<Map<String, String>> {
    matrix_params
        .split(";")
        .map(|pair_str| match pair_str.split_once("=") {
            Some((key, value)) => Ok((
                urlencoding::decode(key)?.into_owned(),
                urlencoding::decode(value)?.into_owned(),
            )),
            _ => Err(anyhow!("Invalid matrix param")),
        })
        .collect::<Result<Map<String, String>>>()
}

pub fn verify_oid(oid: &Cid, uri_str: &str) -> Result<(String, Map<String, String>)> {
    // try to parse as a URI with matrix params
    if &Code::try_from(oid.hash().code())?.digest(uri_str.as_bytes()) == oid.hash()
        && oid.codec() == 0x55
    {
        let first_sc = uri_str.find(";").unwrap_or(uri_str.len());
        Ok((
            // method name
            uri_str
                .get(..first_sc)
                .ok_or(anyhow!("Missing Orbit Method"))?
                .into(),
            // matrix parameters
            get_params(uri_str.get(first_sc + 1..).unwrap_or(""))?,
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

impl Deref for Orbit {
    type Target = OrbitMetadata;
    fn deref(&self) -> &Self::Target {
        &self.metadata
    }
}

impl Orbit {
    pub fn read_delegators(&self) -> &[DIDURL] {
        &self.metadata.read_delegators
    }

    pub fn write_delegators(&self) -> &[DIDURL] {
        &self.metadata.write_delegators
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

#[test]
async fn parameters() -> Result<()> {
    let params = r#"did;did=did%3Akey%3Az6MkqAhhDfRhP8eMWUtk3FjG2nMiXNUGNU5Evsnq89uKNdom;hosts=12D3KooWNmUKqU9EhKKyWdHTyZud8Yj3HWFyf7wSdAe6JudGg4Ly%3A%2Fip4%2F127.0.0.1%2Ftcp%2F8081%2Fp2p%2F12D3KooWG4GKKKocGcX9pfdcdQncaLM73mY4X6TwB6tT48g1ijTY%2Fp2p-circuit%2Fp2p%2F12D3KooWNmUKqU9EhKKyWdHTyZud8Yj3HWFyf7wSdAe6JudGg4Ly;vm=z6MkqAhhDfRhP8eMWUtk3FjG2nMiXNUGNU5Evsnq89uKNdom"#;
    let oid: Cid = "zCT5htkeCSu7WefuBKYUidQJkRgEvEGZQrFVqYS6ZJVM6zwLCRcF".parse()?;
    let _md = get_metadata(&oid, params, &Default::default()).await?;
    Ok(())
}
