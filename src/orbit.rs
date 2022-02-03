use crate::{
    auth::{Action, AuthorizationPolicy, AuthorizationToken},
    cas::ContentAddressedStorage,
    codec::SupportedCodecs,
    config::ExternalApis,
    s3::{Service, Store},
    siwe::{SIWETokens, SIWEZcapTokens},
    tz::TezosAuthorizationString,
    tz_orbit::params_to_tz_orbit,
    zcap::ZCAPTokens,
};
use anyhow::{anyhow, Result};
use ipfs::{IpfsOptions, MultiaddrWithoutPeerId, UninitializedIpfs};
//use ipfs_embed::{generate_keypair, multiaddr::multiaddr, Config, Keypair, Multiaddr, PeerId};
use libipld::cid::{
    multibase::Base,
    multihash::{Code, MultihashDigest},
    Cid,
};
use libp2p::{
    core::Multiaddr,
    identity::{ed25519::Keypair as Ed25519Keypair, Keypair},
    multiaddr::multiaddr,
    swarm::SwarmEvent,
    PeerId,
};
use rocket::{
    futures::TryStreamExt,
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
    future::Future,
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

    pub fn hosts(&self) -> impl Iterator<Item = &PeerId> {
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
    ZCAP(Box<ZCAPTokens>),
    SIWEZcapDelegated(Box<SIWEZcapTokens>),
    SIWEDelegated(Box<SIWETokens>),
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthTokens {
    type Error = anyhow::Error;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let ats =
            if let Outcome::Success(tz) = TezosAuthorizationString::from_request(request).await {
                Self::Tezos(tz)
            } else if let Outcome::Success(siwe) = SIWETokens::from_request(request).await {
                Self::SIWEDelegated(Box::new(siwe))
            } else if let Outcome::Success(siwe) = SIWEZcapTokens::from_request(request).await {
                Self::SIWEZcapDelegated(Box::new(siwe))
            } else if let Outcome::Success(zcap) = ZCAPTokens::from_request(request).await {
                Self::ZCAP(Box::new(zcap))
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
            Self::SIWEDelegated(token) => token.action(),
            Self::SIWEZcapDelegated(token) => token.action(),
        }
    }
    fn target_orbit(&self) -> &Cid {
        match self {
            Self::Tezos(token) => token.target_orbit(),
            Self::ZCAP(token) => token.target_orbit(),
            Self::SIWEDelegated(token) => token.target_orbit(),
            Self::SIWEZcapDelegated(token) => token.target_orbit(),
        }
    }
}
#[rocket::async_trait]
impl AuthorizationPolicy<AuthTokens> for OrbitMetadata {
    async fn authorize(&self, auth_token: &AuthTokens) -> Result<()> {
        match auth_token {
            AuthTokens::Tezos(token) => self.authorize(token).await,
            AuthTokens::ZCAP(token) => self.authorize(token.as_ref()).await,
            AuthTokens::SIWEDelegated(token) => self.authorize(token.as_ref()).await,
            AuthTokens::SIWEZcapDelegated(token) => self.authorize(token.as_ref()).await,
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
struct OrbitTasks {
    ipfs: Arc<AbortOnDrop<()>>,
    new_connections: Arc<AbortOnDrop<()>>,
}

impl OrbitTasks {
    fn new<F: Future<Output = ()> + Send + 'static>(
        ipfs_future: F,
        head_request_receiver: std::sync::mpsc::Receiver<()>,
        store: Store,
    ) -> Self {
        let ipfs = Arc::new(AbortOnDrop::new(tokio::spawn(ipfs_future)));

        let handle = |mut request_receiver: std::sync::mpsc::Receiver<()>, store: Store| async move {
            while let Ok(Ok(returned_receiver)) = tokio::task::spawn_blocking(move || {
                request_receiver.recv().map(|_| request_receiver)
            })
            .await
            {
                if let Err(e) = store.request_heads().await {
                    tracing::error!("failed to request heads from peers: {}", e)
                }
                request_receiver = returned_receiver;
            }
        };

        let new_connections = Arc::new(AbortOnDrop::new(tokio::spawn(handle(
            head_request_receiver,
            store,
        ))));
        Self {
            ipfs,
            new_connections,
        }
    }
}

#[derive(Clone)]
pub struct Orbit {
    pub service: Service,
    metadata: OrbitMetadata,
    tasks: OrbitTasks,
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
        ("tz", ExternalApis { tzkt, .. }) => params_to_tz_orbit(*oid, &params, tzkt).await?,
        _ => OrbitMetadata {
            id: *oid,
            controllers: vec![get_params_vm(method.as_ref(), &params)
                .ok_or_else(|| anyhow!("Missing Implicit Controller Params"))?],
            read_delegators: vec![],
            write_delegators: vec![],
            revocations: vec![],
            hosts: params
                .get("hosts")
                .map(|hs| parse_hosts_str(hs))
                .unwrap_or_else(|| Ok(Default::default()))?,
        },
    })
}

// Using Option to distinguish when the orbit already exists from a hard error
pub async fn create_orbit(
    md: &OrbitMetadata,
    path: PathBuf,
    auth: &[u8],
    relay: (PeerId, Multiaddr),
    keys_lock: &RwLock<Map<PeerId, Ed25519Keypair>>,
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
            .unwrap_or_else(Ed25519Keypair::generate)
    };

    fs::write(dir.join("metadata"), serde_json::to_vec_pretty(md)?).await?;
    fs::write(dir.join("access_log"), auth).await?;
    fs::write(dir.join("kp"), kp.encode()).await?;

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
    load_orbit_(dir, relay).await.map(Some)
}

// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
#[cached(size = 100, result = true, sync_writes = true)]
async fn load_orbit_(dir: PathBuf, relay: (PeerId, Multiaddr)) -> Result<Orbit> {
    let kp = Ed25519Keypair::decode(&mut fs::read(dir.join("kp")).await?)?;
    let local_peer_id = PeerId::from_public_key(ipfs::PublicKey::Ed25519(kp.public()));

    let md: OrbitMetadata = serde_json::from_slice(&fs::read(dir.join("metadata")).await?)?;
    let id = md.id.to_string_of_base(Base::Base58Btc)?;
    tracing::debug!("loading orbit {}, {:?}", &id, &dir);

    let ipfs_opts = IpfsOptions {
        ipfs_path: dir.join("block_store"),
        keypair: Keypair::Ed25519(kp),
        bootstrap: vec![],
        mdns: false,
        kad_protocol: Some(md.id().to_string()),
        listening_addrs: vec![multiaddr!(P2pCircuit)],
        span: None,
    };

    let task_hosts = md.hosts.clone();
    let (head_request_sender, head_request_receiver) = std::sync::mpsc::sync_channel::<()>(100);

    let (ipfs, ipfs_task) = UninitializedIpfs::<ipfs::Types>::new(ipfs_opts)
        .handle_swarm_events(move |swarm, event| {
            if let SwarmEvent::ConnectionEstablished { peer_id, .. } = event {
                if task_hosts.contains_key(peer_id) {
                    if let Err(_) = head_request_sender.send(()) {
                        tracing::error!(
                            "receiver hung up, unable to request heads after connecting to: {}",
                            peer_id
                        );
                    }
                } else {
                    if let Err(()) = swarm.disconnect_peer_id(peer_id.clone()) {
                        tracing::error!(
                            "tried to disconnect from a peer that is not connected: {}",
                            peer_id
                        );
                    };
                    swarm.ban_peer_id(peer_id.clone());
                }
            }
        })
        .start()
        .await?;

    let db = sled::open(dir.join(&id).with_extension("ks3db"))?;

    let service_store = Store::new(id, ipfs.clone(), db)?;
    let service = Service::start(service_store).await?;

    let tasks = OrbitTasks::new(ipfs_task, head_request_receiver, service.store.clone());

    ipfs.connect(MultiaddrWithoutPeerId::try_from(relay.1)?.with(relay.0)).await?;

    tokio_stream::iter(
        md.hosts
            .clone()
            .into_iter()
            .filter(|(p, _)| p != &local_peer_id)
            .map(|(peer, addrs)| addrs.into_iter().zip(std::iter::repeat(peer)))
            .flatten()
            .map(|(addr, peer_id)| Ok(MultiaddrWithoutPeerId::try_from(addr)?.with(peer_id))),
    )
    .try_for_each(|multiaddr| ipfs.connect(multiaddr))
    .await?;

    Ok(Orbit {
        service,
        metadata: md,
        tasks,
    })
}

pub fn parse_hosts_str(s: &str) -> Result<Map<PeerId, Vec<Multiaddr>>> {
    s.split('|')
        .map(|hs| {
            hs.split_once(":")
                .ok_or_else(|| anyhow!("missing host:addrs map"))
                .and_then(|(id, s)| {
                    Ok((
                        id.parse()?,
                        s.split(',')
                            .map(|a| Ok(a.parse()?))
                            .collect::<Result<Vec<Multiaddr>>>()?,
                    ))
                })
        })
        .collect()
}

pub fn get_params(matrix_params: &str) -> Result<Map<String, String>> {
    matrix_params
        .split(';')
        .map(|pair_str| match pair_str.split_once("=") {
            Some((key, value)) => Ok((
                urlencoding::decode(key)?.into_owned(),
                urlencoding::decode(value)?.into_owned(),
            )),
            _ => Err(anyhow!("Invalid matrix param")),
        })
        .collect::<Result<Map<String, String>>>()
}

pub fn hash_same<B: AsRef<[u8]>>(c: &Cid, b: B) -> Result<Cid> {
    Ok(Cid::new_v1(
        c.codec(),
        Code::try_from(c.hash().code())?.digest(b.as_ref()),
    ))
}

pub fn verify_oid(oid: &Cid, uri_str: &str) -> Result<(String, Map<String, String>)> {
    // try to parse as a URI with matrix params
    if &hash_same(oid, uri_str)? == oid && oid.codec() == 0x55 {
        let first_sc = uri_str.find(';').unwrap_or(uri_str.len());
        Ok((
            // method name
            uri_str
                .get(..first_sc)
                .ok_or_else(|| anyhow!("Missing Orbit Method"))?
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
