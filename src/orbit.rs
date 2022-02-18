use crate::{
    auth::{Action, AuthorizationPolicy, AuthorizationToken},
    cas::ContentAddressedStorage,
    codec::SupportedCodecs,
    config::ExternalApis,
    ipfs::create_ipfs,
    s3::{behaviour::BehaviourProcess, Service, Store},
    siwe::{SIWETokens, SIWEZcapTokens},
    tz::TezosAuthorizationString,
    zcap::ZCAPTokens,
};
use anyhow::{anyhow, Result};
use ipfs::MultiaddrWithoutPeerId;
use libipld::cid::{
    multibase::Base,
    multihash::{Code, MultihashDigest},
    Cid,
};
use libp2p::{
    core::Multiaddr,
    identity::{ed25519::Keypair as Ed25519Keypair, Keypair},
    PeerId,
};
use rocket::{
    futures::TryStreamExt,
    http::Status,
    request::{FromRequest, Outcome, Request},
    tokio::{fs, task::JoinHandle},
};

use cached::proc_macro::cached;
use ssi::{
    did::{Document, RelativeDIDURL, ServiceEndpoint, VerificationMethod, DIDURL},
    did_resolve::DIDResolver,
};
use std::{
    collections::HashMap as Map,
    convert::TryFrom,
    ops::Deref,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, RwLock},
};
use tokio::spawn;

/// An implementation of an Orbit Manifest.
///
/// Orbit Manifests are [DID Documents](https://www.w3.org/TR/did-spec-registries/#did-methods) used directly as the root of a capabilities
/// authorization framework. This enables Orbits to be managed using independant DID lifecycle management tools.
#[derive(Clone, Debug)]
pub struct Manifest {
    id: String,
    delegators: Vec<DIDURL>,
    invokers: Vec<DIDURL>,
    bootstrap_peers: Vec<Peer>,
}

#[derive(Clone, Debug)]
pub struct Peer {
    pub id: PeerId,
    pub addrs: Vec<Multiaddr>,
}

fn id_from_vm(did: &str, vm: VerificationMethod) -> DIDURL {
    match vm {
        VerificationMethod::DIDURL(d) => d,
        VerificationMethod::RelativeDIDURL(f) => f.to_absolute(did),
        VerificationMethod::Map(m) => {
            if let Ok(abs_did_url) = DIDURL::from_str(&m.id) {
                abs_did_url
            } else if let Ok(rel_did_url) = RelativeDIDURL::from_str(&m.id) {
                rel_did_url.to_absolute(did)
            } else {
                // HACK well-behaved did methods should not allow id's which lead to this path
                DIDURL {
                    did: m.id,
                    ..Default::default()
                }
            }
        }
    }
}

impl From<Document> for Manifest {
    fn from(
        Document {
            id,
            capability_delegation,
            capability_invocation,
            verification_method,
            service,
            ..
        }: Document,
    ) -> Self {
        Self {
            delegators: capability_delegation
                .or_else(|| verification_method.clone())
                .unwrap_or_else(|| vec![])
                .into_iter()
                .map(|vm| id_from_vm(&id, vm))
                .collect(),
            invokers: capability_invocation
                .or(verification_method)
                .unwrap_or_else(|| vec![])
                .into_iter()
                .map(|vm| id_from_vm(&id, vm))
                .collect(),
            bootstrap_peers: service
                .unwrap_or_else(|| vec![])
                .into_iter()
                .filter(|s| s.type_.any(|s| s == "KeplerOrbitPeer"))
                .filter_map(|s| {
                    Some(Peer {
                        id: s.id[1..].parse().ok()?,
                        addrs: s
                            .service_endpoint?
                            .into_iter()
                            .filter_map(|e| match e {
                                ServiceEndpoint::URI(a) => match &a.get(..10) {
                                    Some("multiaddr:") => a.get(10..)?.parse().ok(),
                                    _ => None,
                                },
                                _ => None,
                            })
                            .collect(),
                    })
                })
                .collect(),
            id,
        }
    }
}

pub async fn resolve(id: &str) -> anyhow::Result<Option<Manifest>> {
    let (md, doc, doc_md) = didkit::DID_METHODS.resolve(id, &Default::default()).await;

    match (md.error, doc, doc_md.and_then(|d| d.deactivated)) {
        (Some(e), _, _) => Err(anyhow!(e)),
        (_, _, Some(true)) | (_, None, _) => Ok(None),
        (None, Some(d), None) | (None, Some(d), Some(false)) => Ok(Some(d.into())),
    }
}

impl Manifest {
    /// ID of the Orbit, usually a DID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The set of Peers discoverable from the Orbit Manifest.
    pub fn bootstrap_peers(&self) -> &[Peer] {
        &self.bootstrap_peers
    }

    /// The set of [Verification Methods](https://www.w3.org/TR/did-core/#verification-methods) who are authorized to delegate any capability.
    pub fn delegators(&self) -> &[DIDURL] {
        &self.delegators
    }

    /// The set of [Verification Methods](https://www.w3.org/TR/did-core/#verification-methods) who are authorized to invoke any capability.
    pub fn invokers(&self) -> &[DIDURL] {
        &self.invokers
    }

    pub fn make_uri(&self, cid: &Cid) -> Result<String> {
        Ok(format!(
            "kepler:{}/ipfs/{}",
            self.id(),
            cid.to_string_of_base(Base::Base58Btc)?
        ))
    }
}

pub enum AuthTokens {
    Tezos(Box<TezosAuthorizationString>),
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
                Self::Tezos(Box::new(tz))
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
    fn target_orbit(&self) -> &str {
        match self {
            Self::Tezos(token) => token.target_orbit(),
            Self::ZCAP(token) => token.target_orbit(),
            Self::SIWEDelegated(token) => token.target_orbit(),
            Self::SIWEZcapDelegated(token) => token.target_orbit(),
        }
    }
}
#[rocket::async_trait]
impl AuthorizationPolicy<AuthTokens> for Manifest {
    async fn authorize(&self, auth_token: &AuthTokens) -> Result<()> {
        match auth_token {
            AuthTokens::Tezos(token) => self.authorize(token.as_ref()).await,
            AuthTokens::ZCAP(token) => self.authorize(token.as_ref()).await,
            AuthTokens::SIWEDelegated(token) => self.authorize(token.as_ref()).await,
            AuthTokens::SIWEZcapDelegated(token) => self.authorize(token.as_ref()).await,
        }
    }
}

#[derive(Debug)]
pub struct AbortOnDrop<T>(JoinHandle<T>);

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

#[derive(Clone, Debug)]
struct OrbitTasks {
    _ipfs: Arc<AbortOnDrop<()>>,
    _behaviour_process: BehaviourProcess,
}

impl OrbitTasks {
    fn new(ipfs_future: JoinHandle<()>, behaviour_process: BehaviourProcess) -> Self {
        let ipfs = Arc::new(AbortOnDrop::new(ipfs_future));
        Self {
            _ipfs: ipfs,
            _behaviour_process: behaviour_process,
        }
    }
}

#[derive(Clone)]
pub struct Orbit {
    pub service: Service,
    _tasks: OrbitTasks,
    metadata: Manifest,
}

// Using Option to distinguish when the orbit already exists from a hard error
pub async fn create_orbit(
    id: &str,
    path: PathBuf,
    auth: &[u8],
    relay: (PeerId, Multiaddr),
    keys_lock: &RwLock<Map<PeerId, Ed25519Keypair>>,
) -> Result<Option<Orbit>> {
    let md = match resolve(id).await? {
        Some(m) => m,
        _ => return Ok(None),
    };
    let dir = path.join(id);

    // fails if DIR exists, this is Create, not Open
    if dir.exists() {
        return Ok(None);
    }
    fs::create_dir(&dir)
        .await
        .map_err(|e| anyhow!("Couldn't create dir: {}", e))?;

    let kp = {
        let mut keys = keys_lock.write().map_err(|e| anyhow!(e.to_string()))?;
        md.bootstrap_peers()
            .iter()
            .find_map(|p| keys.remove(&p.id))
            .unwrap_or_else(generate_keypair)
    };

    fs::write(dir.join("access_log"), auth).await?;
    fs::write(dir.join("kp"), kp.encode()).await?;

    Ok(Some(load_orbit(md.id, path, relay).await.map(|o| {
        o.ok_or_else(|| anyhow!("Couldn't find newly created orbit"))
    })??))
}

pub async fn load_orbit(
    id: String,
    path: PathBuf,
    relay: (PeerId, Multiaddr),
) -> Result<Option<Orbit>> {
    let dir = path.join(&id);
    if !dir.exists() {
        return Ok(None);
    }
    load_orbit_(dir, id, relay).await.map(Some)
}

// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
#[cached(size = 100, result = true, sync_writes = true)]
async fn load_orbit_(dir: PathBuf, id: String, relay: (PeerId, Multiaddr)) -> Result<Orbit> {
    let md = resolve(&id)
        .await?
        .ok_or_else(|| anyhow!("Orbit DID Document not resolvable"))?;
    let kp = Keypair::from_bytes(&fs::read(dir.join("kp")).await?)?;
    let mut cfg = Config::new(&dir.join("block_store"), kp);
    cfg.network.streams = None;

    tracing::debug!("loading orbit {}, {:?}", &id, &dir);

    let (ipfs, ipfs_future, receiver) = create_ipfs(
        id.clone(),
        &dir,
        Keypair::Ed25519(kp),
        md.hosts.clone().into_keys(),
    )
    .await?;

    let ipfs_task = spawn(ipfs_future);
    ipfs.connect(MultiaddrWithoutPeerId::try_from(relay.1)?.with(relay.0))
        .await?;

    let db = sled::open(dir.join(&id).with_extension("ks3db"))?;

    let service_store = Store::new(id, ipfs.clone(), db)?;
    let service = Service::start(service_store).await?;

    let behaviour_process = BehaviourProcess::new(service.store.clone(), receiver);

    let tasks = OrbitTasks::new(ipfs_task, behaviour_process);

    tokio_stream::iter(
        md.hosts
            .clone()
            .into_iter()
            .filter(|(p, _)| p != &local_peer_id)
            .flat_map(|(peer, addrs)| addrs.into_iter().zip(std::iter::repeat(peer)))
            .map(|(addr, peer_id)| Ok(MultiaddrWithoutPeerId::try_from(addr)?.with(peer_id))),
    )
    .try_for_each(|multiaddr| ipfs.connect(multiaddr))
    .await?;

    Ok(Orbit {
        service,
        metadata: md,
        _tasks: tasks,
    })
}

pub fn hash_same<B: AsRef<[u8]>>(c: &Cid, b: B) -> Result<Cid> {
    Ok(Cid::new_v1(
        c.codec(),
        Code::try_from(c.hash().code())?.digest(b.as_ref()),
    ))
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
    type Target = Manifest;
    fn deref(&self) -> &Self::Target {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use didkit::DID_METHODS;
    use ssi::{
        did::Source,
        jwk::{Algorithm, Params, JWK},
    };
    #[test]
    async fn manifest_resolution() {
        let j = JWK::generate_secp256k1().unwrap();
        let did = DID_METHODS
            .generate(&Source::KeyAndPattern(&j, "pkh:tz"))
            .unwrap();

        let md = resolve(&did).await.unwrap().unwrap();
        println!("{:?}", md);
        assert!(false);
    }
}
