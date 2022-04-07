use crate::{
    auth::AuthorizationToken,
    capabilities::{store::Store as CapStore, AuthRef, Invoke, Service as CapService},
    cas::ContentAddressedStorage,
    codec::SupportedCodecs,
    config,
    ipfs::create_ipfs,
    manifest::Manifest,
    resource::{OrbitId, ResourceId},
    s3::{behaviour::BehaviourProcess, Service as KVService, Store},
    siwe::{SIWETokens, SIWEZcapTokens},
    tz::TezosAuthorizationString,
    zcap::ZCAPTokens,
};
use anyhow::{anyhow, Result};
use ipfs::{MultiaddrWithPeerId, MultiaddrWithoutPeerId};
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
    tokio::task::JoinHandle,
};

use cached::proc_macro::cached;
use std::{
    collections::HashMap as Map,
    convert::TryFrom,
    ops::Deref,
    sync::{Arc, RwLock},
};
use tokio::spawn;

use super::storage::StorageUtils;

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
    pub service: KVService,
    _tasks: OrbitTasks,
    pub manifest: Manifest,
    pub capabilities: CapService,
}

impl Orbit {
    async fn new(
        config: &config::Config,
        kp: Ed25519Keypair,
        manifest: Manifest,
        relay: Option<(PeerId, Multiaddr)>,
    ) -> anyhow::Result<Self> {
        let id = manifest.id().get_cid().to_string_of_base(Base::Base58Btc)?;
        let local_peer_id = PeerId::from_public_key(&ipfs::PublicKey::Ed25519(kp.public()));
        let (ipfs, ipfs_future, receiver) = create_ipfs(
            id,
            config,
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

        let path = match &config.storage.indexes {
            config::IndexStorage::Local(r) => &r.path,
            _ => panic!("To be refactored."),
        };
        let db = sled::open(path.join(&id.to_string()).with_extension("ks3db"))?;

        let service_store = Store::new(id.clone(), ipfs.clone(), db)?;
        let service = KVService::start(service_store).await?;

        let cap_db = sled::open(path.as_ref().join(&id).with_extension("capdb"))?;
        let cap_store = CapStore::new(manifest.id(), ipfs.clone(), &cap_db)?;
        let capabilities = CapService::start(cap_store).await?;

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
            capabilities,
        })
    }

    pub async fn connect(&self, node: MultiaddrWithPeerId) -> anyhow::Result<()> {
        self.service.store.ipfs.connect(node).await
    }
}

// Using Option to distinguish when the orbit already exists from a hard error
pub async fn create_orbit(
    id: &OrbitId,
    config: &config::Config,
    auth: &[u8],
    relay: (PeerId, Multiaddr),
    keys_lock: &RwLock<Map<PeerId, Ed25519Keypair>>,
) -> Result<Option<Orbit>> {
    let md = match Manifest::resolve_dyn(id, None).await? {
        Some(m) => m,
        _ => return Ok(None),
    };

    // fails if DIR exists, this is Create, not Open
    let storage_utils = StorageUtils::new(config.storage.blocks.clone());
    if storage_utils.exists(id.get_cid()).await? {
        return Ok(None);
    }

    let kp = {
        let mut keys = keys_lock.write().map_err(|e| anyhow!(e.to_string()))?;
        md.bootstrap_peers()
            .peers
            .iter()
            .find_map(|p| keys.remove(&p.id))
            .unwrap_or_else(Ed25519Keypair::generate)
    };

    storage_utils.setup_orbit(id.clone(), kp, auth).await?;

    Ok(Some(
        load_orbit(md.id().get_cid(), config, relay)
            .await
            .map(|o| o.ok_or_else(|| anyhow!("Couldn't find newly created orbit")))??,
    ))
}

pub async fn load_orbit(
    id_cid: Cid,
    config: &config::Config,
    relay: (PeerId, Multiaddr),
) -> Result<Option<Orbit>> {
    let storage_utils = StorageUtils::new(config.storage.blocks.clone());
    if !storage_utils.exists(id_cid).await? {
        return Ok(None);
    }
    load_orbit_inner(id_cid, config.clone(), relay)
        .await
        .map(Some)
}

// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
#[cached(size = 100, result = true, sync_writes = true)]
async fn load_orbit_inner(
    orbit: Cid,
    config: config::Config,
    relay: (PeerId, Multiaddr),
) -> Result<Orbit> {
    let storage_utils = StorageUtils::new(config.storage.blocks.clone());
    let id = storage_utils
        .orbit_id(orbit)
        .await?
        .ok_or_else(|| anyhow!("Orbit `{}` doesn't have its orbit URL stored.", orbit))?;

    let md = Manifest::resolve_dyn(&id, None)
        .await?
        .ok_or_else(|| anyhow!("Orbit DID Document not resolvable"))?;

    // let kp = Ed25519Keypair::decode(&mut fs::read(dir.join("kp")).await?)?;
    let kp = storage_utils.key_pair(orbit).await?.unwrap();

    debug!("loading orbit {}", &id);

    let orbit = Orbit::new(&config, kp, md, Some(relay)).await?;
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

#[rocket::async_trait]
impl Invoke<AuthTokens> for Orbit {
    async fn invoke(&self, invocation: &AuthTokens) -> anyhow::Result<AuthRef> {
        match invocation {
            AuthTokens::Tezos(token) => self.invoke(token.as_ref()).await,
            AuthTokens::ZCAP(token) => self.invoke(token.as_ref()).await,
            AuthTokens::SIWEDelegated(token) => self.invoke(token.as_ref()).await,
            AuthTokens::SIWEZcapDelegated(token) => self.invoke(token.as_ref()).await,
        }
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
        let dir = TempDir::new(&md.id().get_cid().to_string())
            .unwrap()
            .path()
            .to_path_buf();
        let config = config::Config {
            storage: config::Storage {
                blocks: config::BlockStorage::Local(config::LocalBlockStorage {
                    path: dir.clone(),
                }),
                indexes: config::IndexStorage::Local(config::LocalIndexStorage {
                    path: dir.clone(),
                }),
            },
            ..Default::default()
        };
        Orbit::new(&config, Ed25519Keypair::generate(), md, None).await
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

        let _orbit = op(md).await.unwrap();
    }
}
