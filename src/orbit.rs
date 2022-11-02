use crate::{
    capabilities::{store::Store as CapStore, Service as CapService},
    config,
    kv::{behaviour::BehaviourProcess, Service as KVService, Store},
    manifest::Manifest,
    storage::{ImmutableStore, StorageConfig},
    BlockConfig, BlockStores,
};
use anyhow::{anyhow, Result};
use derive_builder::Builder;
use kepler_lib::libipld::cid::{
    multihash::{Code, MultihashDigest},
    Cid,
};
use kepler_lib::resource::OrbitId;
use libp2p::{
    core::Multiaddr,
    identity::{ed25519::Keypair as Ed25519Keypair, PublicKey},
    PeerId,
};
use rocket::tokio::task::JoinHandle;

use cached::proc_macro::cached;
use std::{convert::TryFrom, error::Error as StdError, ops::Deref};

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
    _behaviour_process: BehaviourProcess,
}

impl OrbitTasks {
    fn new(behaviour_process: BehaviourProcess) -> Self {
        Self {
            _behaviour_process: behaviour_process,
        }
    }
}

#[derive(Clone)]
pub struct Orbit<B> {
    pub service: KVService<B>,
    pub manifest: Manifest,
    pub capabilities: CapService<B>,
}

#[derive(Clone, Debug, Builder)]
pub struct OrbitPeerConfig<B, I = config::IndexStorage> {
    #[builder(setter(into))]
    identity: Ed25519Keypair,
    #[builder(setter(into))]
    manifest: Manifest,
    #[builder(setter(into))]
    relay: Option<(PeerId, Multiaddr)>,
    #[builder(setter(into))]
    blocks: B,
    #[builder(setter(into))]
    index: I,
}

impl<B> Orbit<B>
where
    B: ImmutableStore + Clone,
{
    async fn open<C>(config: &OrbitPeerConfig<C>) -> anyhow::Result<Option<Self>>
    where
        C: StorageConfig<B>,
        C::Error: 'static + Sync + Send,
        B::Error: 'static,
    {
        let id = config.manifest.id().get_cid();
        let local_peer_id = PeerId::from_public_key(&PublicKey::Ed25519(config.identity.public()));

        let blocks = match config.blocks.open(config.manifest.id()).await? {
            Some(b) => b,
            None => return Ok(None),
        };
        let service_store = Store::new(id, blocks.clone(), config.index.clone()).await?;
        let service = KVService::start(service_store).await?;

        let cap_store =
            CapStore::new(config.manifest.id(), blocks.clone(), config.index.clone()).await?;
        let capabilities = CapService::start(cap_store).await?;

        Ok(Some(Orbit {
            service,
            manifest: config.manifest.clone(),
            capabilities,
        }))
    }

    async fn create<C>(config: &OrbitPeerConfig<C>) -> anyhow::Result<Self>
    where
        C: StorageConfig<B>,
        C::Error: 'static + Sync + Send,
        B::Error: 'static,
    {
        let id = config.manifest.id().get_cid();
        let local_peer_id = PeerId::from_public_key(&PublicKey::Ed25519(config.identity.public()));

        let blocks = config.blocks.create(config.manifest.id()).await?;
        let service_store = Store::new(id, blocks.clone(), config.index.clone()).await?;
        let service = KVService::start(service_store).await?;

        let cap_store =
            CapStore::new(config.manifest.id(), blocks.clone(), config.index.clone()).await?;
        let capabilities = CapService::start(cap_store).await?;

        Ok(Orbit {
            service,
            manifest: config.manifest.clone(),
            capabilities,
        })
    }
}

#[async_trait]
pub trait ProviderUtils {
    type Error: StdError;
    async fn exists(&self, orbit: &OrbitId) -> Result<bool, Self::Error>;
    async fn relay_key_pair(&self) -> Result<Ed25519Keypair, Self::Error>;
    async fn key_pair(&self, orbit: &OrbitId) -> Result<Option<Ed25519Keypair>, Self::Error>;
    async fn setup_orbit(&self, orbit: &OrbitId, key: &Ed25519Keypair) -> Result<(), Self::Error>;
}

// Using Option to distinguish when the orbit already exists from a hard error
pub async fn create_orbit(
    id: &OrbitId,
    store_config: &BlockConfig,
    index_config: &config::IndexStorage,
    relay: (PeerId, Multiaddr),
    kp: Ed25519Keypair,
) -> Result<Option<Orbit<BlockStores>>> {
    let md = match Manifest::resolve_dyn(id, None).await? {
        Some(m) => m,
        _ => return Ok(None),
    };

    // fails if DIR exists, this is Create, not Open
    if store_config.exists(&id).await? {
        return Ok(None);
    }

    store_config.setup_orbit(&id, &kp).await?;

    Ok(Some(
        load_orbit(id.clone(), store_config, index_config, relay)
            .await
            .map(|o| o.ok_or_else(|| anyhow!("Couldn't find newly created orbit")))??,
    ))
}

pub async fn load_orbit(
    orbit: OrbitId,
    store_config: &BlockConfig,
    index_config: &config::IndexStorage,
    relay: (PeerId, Multiaddr),
) -> Result<Option<Orbit<BlockStores>>> {
    if !store_config.exists(&orbit).await? {
        return Ok(None);
    }
    load_orbit_inner(orbit, store_config.clone(), index_config.clone(), relay)
        .await
        .map(Some)
}

// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
#[cached(size = 100, result = true, sync_writes = true)]
async fn load_orbit_inner(
    orbit: OrbitId,
    store_config: BlockConfig,
    index_config: config::IndexStorage,
    relay: (PeerId, Multiaddr),
) -> Result<Orbit<BlockStores>> {
    debug!("loading orbit {}", &orbit);
    Orbit::open(
        &OrbitPeerConfigBuilder::<BlockConfig, config::IndexStorage>::default()
            .manifest(
                Manifest::resolve_dyn(&orbit, None)
                    .await?
                    .ok_or_else(|| anyhow!("Orbit DID Document not resolvable"))?,
            )
            .identity(
                store_config
                    .key_pair(&orbit)
                    .await?
                    .ok_or_else(|| anyhow!("Peer Identity key could not be found"))?,
            )
            .blocks(store_config)
            .index(index_config)
            .relay(relay)
            .build()?,
    )
    .await?
    .ok_or_else(|| anyhow!("Orbit could not be opened: not found"))
}

pub fn hash_same<B: AsRef<[u8]>>(c: &Cid, b: B) -> Result<Cid> {
    Ok(Cid::new_v1(
        c.codec(),
        Code::try_from(c.hash().code())?.digest(b.as_ref()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kepler_lib::resolver::DID_METHODS;
    use kepler_lib::ssi::{
        did::{Source, DIDURL},
        jwk::JWK,
    };
    use std::convert::TryInto;
    use tempdir::TempDir;

    async fn op(md: Manifest) -> anyhow::Result<Orbit<BlockStores>> {
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
