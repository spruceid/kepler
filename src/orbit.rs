use crate::{
    capabilities::{store::Store as CapStore, Service as CapService},
    cas::ContentAddressedStorage,
    codec::SupportedCodecs,
    config,
    ipfs::create_ipfs,
    kv::{behaviour::BehaviourProcess, Service as KVService, Store},
    manifest::Manifest,
};
use anyhow::{anyhow, Result};
use ipfs::{MultiaddrWithPeerId, MultiaddrWithoutPeerId};
use lib::resource::OrbitId;
use libipld::cid::{
    multihash::{Code, MultihashDigest},
    Cid,
};
use libp2p::{
    core::Multiaddr,
    identity::{ed25519::Keypair as Ed25519Keypair, Keypair},
    PeerId,
};
use rocket::{futures::TryStreamExt, tokio::task::JoinHandle};

use cached::proc_macro::cached;
use std::{convert::TryFrom, ops::Deref, sync::Arc};
use tokio::spawn;

use super::storage::StorageUtils;

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
        let id = manifest.id().get_cid();
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

        let service_store = Store::new(id, ipfs.clone(), config.storage.indexes.clone()).await?;
        let service = KVService::start(service_store).await?;

        let cap_store =
            CapStore::new(manifest.id(), ipfs.clone(), config.storage.indexes.clone()).await?;
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
    kp: Ed25519Keypair,
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

#[cfg(test)]
mod tests {
    use super::*;
    use lib::didkit::DID_METHODS;
    use lib::ssi::{
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
