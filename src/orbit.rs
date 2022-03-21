use crate::{
    auth::AuthorizationToken,
    cas::ContentAddressedStorage,
    codec::SupportedCodecs,
    ipfs::create_ipfs,
    manifest::Manifest,
    resource::{OrbitId, ResourceId},
    s3::{behaviour::BehaviourProcess, Service, Store},
    siwe::{SIWETokens, SIWEZcapTokens},
    tz::TezosAuthorizationString,
    zcap::ZCAPTokens,
};
use anyhow::{anyhow, Result};
use ipfs::{MultiaddrWithPeerId, MultiaddrWithoutPeerId};
use libipld::cid::{
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
use std::{
    collections::HashMap as Map,
    convert::TryFrom,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
use tokio::spawn;

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
    fn resource(&self) -> &ResourceId {
        match self {
            Self::Tezos(token) => token.resource(),
            Self::ZCAP(token) => token.resource(),
            Self::SIWEDelegated(token) => token.resource(),
            Self::SIWEZcapDelegated(token) => token.resource(),
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
    manifest: Manifest,
}

impl Orbit {
    async fn new<P: AsRef<Path>>(
        path: P,
        kp: Ed25519Keypair,
        manifest: Manifest,
        relay: Option<(PeerId, Multiaddr)>,
    ) -> anyhow::Result<Self> {
        let id = manifest.id().get_cid().to_string();
        let local_peer_id = PeerId::from_public_key(&ipfs::PublicKey::Ed25519(kp.public()));
        let (ipfs, ipfs_future, receiver) = create_ipfs(
            id.clone(),
            path.as_ref(),
            Keypair::Ed25519(kp),
            manifest
                .bootstrap_peers()
                .peers
                .iter()
                .map(|p| p.id)
                .collect::<Vec<PeerId>>(),
        )
        .await?;

        let ipfs_task = spawn(ipfs_future);
        if let Some(r) = relay {
            ipfs.connect(MultiaddrWithoutPeerId::try_from(r.1)?.with(r.0))
                .await?;
        };

        let db = sled::open(path.as_ref().join(&id).with_extension("ks3db"))?;

        let service_store = Store::new(id, ipfs.clone(), db)?;
        let service = Service::start(service_store).await?;

        let behaviour_process = BehaviourProcess::new(service.store.clone(), receiver);

        let tasks = OrbitTasks::new(ipfs_task, behaviour_process);

        tokio_stream::iter(
            manifest
                .bootstrap_peers()
                .peers
                .iter()
                .filter(|p| p.id != local_peer_id)
                .flat_map(|peer| {
                    peer.addrs
                        .clone()
                        .into_iter()
                        .zip(std::iter::repeat(peer.id))
                })
                .map(|(addr, peer_id)| Ok(MultiaddrWithoutPeerId::try_from(addr)?.with(peer_id))),
        )
        .try_for_each(|multiaddr| ipfs.connect(multiaddr))
        .await?;

        Ok(Orbit {
            service,
            manifest,
            _tasks: tasks,
        })
    }

    pub async fn connect(&self, node: MultiaddrWithPeerId) -> anyhow::Result<()> {
        self.service.store.ipfs.connect(node).await
    }
}

// Using Option to distinguish when the orbit already exists from a hard error
pub async fn create_orbit(
    id: &OrbitId,
    path: PathBuf,
    auth: &[u8],
    relay: (PeerId, Multiaddr),
    keys_lock: &RwLock<Map<PeerId, Ed25519Keypair>>,
) -> Result<Option<Orbit>> {
    let md = match Manifest::resolve_dyn(id, None).await? {
        Some(m) => m,
        _ => return Ok(None),
    };
    let dir = path.join(&id.get_cid().to_string());

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
            .peers
            .iter()
            .find_map(|p| keys.remove(&p.id))
            .unwrap_or_else(Ed25519Keypair::generate)
    };

    fs::write(dir.join("access_log"), auth).await?;
    fs::write(dir.join("kp"), kp.encode()).await?;
    fs::write(dir.join("id"), id.to_string()).await?;

    Ok(Some(
        load_orbit(md.id().get_cid(), path, relay)
            .await
            .map(|o| o.ok_or_else(|| anyhow!("Couldn't find newly created orbit")))??,
    ))
}

pub async fn load_orbit(
    id_cid: Cid,
    path: PathBuf,
    relay: (PeerId, Multiaddr),
) -> Result<Option<Orbit>> {
    let dir = path.join(&id_cid.to_string());
    if !dir.exists() {
        return Ok(None);
    }
    load_orbit_(dir, relay).await.map(Some)
}

// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
#[cached(size = 100, result = true, sync_writes = true)]
async fn load_orbit_(dir: PathBuf, relay: (PeerId, Multiaddr)) -> Result<Orbit> {
    let id: OrbitId = String::from_utf8(fs::read(dir.join("id")).await?)?.parse()?;
    let md = Manifest::resolve_dyn(&id, None)
        .await?
        .ok_or_else(|| anyhow!("Orbit DID Document not resolvable"))?;

    let kp = Ed25519Keypair::decode(&mut fs::read(dir.join("kp")).await?)?;

    tracing::debug!("loading orbit {}, {:?}", &id, &dir);

    let orbit = Orbit::new(dir, kp, md, Some(relay)).await?;
    Ok(orbit)
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
        &self.manifest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use didkit::DID_METHODS;
    use ssi::{
        did::{Source, DIDURL},
        jwk::JWK,
    };
    use std::convert::TryInto;
    use tempdir::TempDir;

    async fn op(md: Manifest) -> anyhow::Result<Orbit> {
        Orbit::new(
            TempDir::new(&md.id().get_cid().to_string()).unwrap(),
            Ed25519Keypair::generate(),
            md,
            None,
        )
        .await
    }

    #[test]
    async fn did_orbit() {
        let j = JWK::generate_secp256k1().unwrap();
        let did = DID_METHODS
            .generate(&Source::KeyAndPattern(&j, "pkh:tz"))
            .unwrap();
        let oid = DIDURL {
            did,
            fragment: Some("dummy".into()),
            query: None,
            path_abempty: "".into(),
        }
        .try_into()
        .unwrap();

        let md = Manifest::resolve_dyn(&oid, None).await.unwrap().unwrap();

        let orbit = op(md).await.unwrap();
    }
}
