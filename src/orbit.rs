use crate::{manifest::Manifest, BlockConfig, BlockStage, BlockStores};
use anyhow::{anyhow, Result};
use derive_builder::Builder;
use kepler_core::{
    hash::Hash,
    models::kv::Metadata,
    sea_orm,
    storage::{Content, ImmutableReadStore, StorageConfig},
    OrbitDatabase,
};
use kepler_lib::resource::OrbitId;
use libp2p::{
    core::Multiaddr,
    identity::{ed25519::Keypair as Ed25519Keypair, PublicKey},
    PeerId,
};
use rocket::tokio::task::JoinHandle;
use sea_orm::query::Condition;

use cached::proc_macro::cached;
use std::{error::Error as StdError, ops::Deref};

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

#[derive(Clone)]
pub struct Orbit<B, S> {
    pub manifest: Manifest,
    pub capabilities: OrbitDatabase,
    pub store: B,
    pub staging: S,
}

#[derive(Clone, Debug, Builder)]
pub struct OrbitPeerConfig<B, S> {
    #[builder(setter(into))]
    identity: Ed25519Keypair,
    #[builder(setter(into))]
    manifest: Manifest,
    #[builder(setter(into, strip_option), default)]
    relay: Option<(PeerId, Multiaddr)>,
    #[builder(setter(into))]
    store: B,
    #[builder(setter(into))]
    staging: S,
    #[builder(setter(into))]
    db: sea_orm::ConnectOptions,
}

impl<B, S> Orbit<B, S> {
    async fn open<CB, CS>(config: &OrbitPeerConfig<CB, CS>) -> anyhow::Result<Option<Self>>
    where
        CB: StorageConfig<B>,
        CB::Error: 'static + Sync + Send,
        CS: StorageConfig<S>,
        CS::Error: 'static + Sync + Send,
    {
        let id = config.manifest.id().get_cid();
        let _local_peer_id = PeerId::from_public_key(&PublicKey::Ed25519(config.identity.public()));
        let _relay = &config.relay;

        let store = match config.store.open(config.manifest.id()).await? {
            Some(b) => b,
            None => return Ok(None),
        };
        let staging = match config.staging.open(config.manifest.id()).await? {
            Some(b) => b,
            None => return Ok(None),
        };

        let capabilities =
            OrbitDatabase::new(config.db.clone(), config.manifest.id().clone()).await?;

        Ok(Some(Orbit {
            manifest: config.manifest.clone(),
            capabilities,
            store,
            staging,
        }))
    }

    async fn create<CB, CS>(config: &OrbitPeerConfig<CS, CB>) -> anyhow::Result<Self>
    where
        CB: StorageConfig<B>,
        CB::Error: 'static + Sync + Send,
        CS: StorageConfig<S>,
        CS::Error: 'static + Sync + Send,
    {
        let id = config.manifest.id().get_cid();
        let _local_peer_id = PeerId::from_public_key(&PublicKey::Ed25519(config.identity.public()));
        let _relay = &config.relay;

        let store = config.blocks.create(config.manifest.id()).await?;
        let staging = config.staging.create(config.manifest.id()).await?;

        let capabilities =
            OrbitDatabase::new(config.db.clone(), config.manifest.id().clone()).await?;

        Ok(Orbit {
            manifest: config.manifest.clone(),
            capabilities,
            store,
            staging,
        })
    }
}

impl<B, S> Orbit<B, S>
where
    B: ImmutableReadStore,
{
    async fn get(
        &self,
        key: &str,
        version: Option<(u64, Hash)>,
    ) -> anyhow::Result<Option<(Content<B::Readable>, Metadata)>> {
        use kepler_core::models::*;
        // get content id for key from db
        let (key_hash, md): Hash = kv::Entity::find()
            .filter(Condition::all().add(kv::Column::Key.eq(key)))
            .order_by_desc(kv::Column::Seq)
            .order_by_desc(kv::Column::EpochId)
            .one(self.capabilities.readable())
            .await?
            .try_into()?;

        Ok((self.store.read(&key_hash).await?, md))
    }

    async fn list(
        &self,
        prefix: &str,
        version: Option<(u64, Hash)>,
    ) -> anyhow::Result<Vec<String>> {
        use kepler_core::models::*;
        // get content id for key from db
        Ok(kv::Entity::find()
            .filter(Condition::all().add(kv::Column::Key.like(format!("{prefix}%"))))
            .order_by_desc(kv::Column::Seq)
            .order_by_desc(kv::Column::EpochId)
            .one(self.capabilities.readable())
            .await?
            .dedup())
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
    staging_config: &BlockStage,
    db: &str,
    relay: (PeerId, Multiaddr),
    kp: Ed25519Keypair,
) -> Result<Option<Orbit<BlockStores, BlockStores>>> {
    match Manifest::resolve_dyn(id, None).await? {
        Some(_) => {}
        _ => return Ok(None),
    };

    // fails if DIR exists, this is Create, not Open
    if store_config.exists(id).await? {
        return Ok(None);
    }

    // TODO allow using sql db as peer identity store
    store_config.setup_orbit(id, &kp).await?;

    Orbit::create(
        &OrbitPeerConfigBuilder::<BlockConfig>::default()
            .manifest(
                Manifest::resolve_dyn(id, None)
                    .await?
                    .ok_or_else(|| anyhow!("Orbit DID Document not resolvable"))?,
            )
            .identity(
                store_config
                    .key_pair(id)
                    .await?
                    .ok_or_else(|| anyhow!("Peer Identity key could not be found"))?,
            )
            .blocks(store_config.clone())
            .staging(staging_config.clone())
            .db(db)
            .relay(relay.clone())
            .build()?,
    )
    .await?;

    Ok(Some(
        load_orbit(id.clone(), store_config, staging_config, db, relay)
            .await
            .map(|o| o.ok_or_else(|| anyhow!("Couldn't find newly created orbit")))??,
    ))
}

pub async fn load_orbit(
    orbit: OrbitId,
    store_config: &BlockConfig,
    stage_config: &BlockStage,
    db: &str,
    relay: (PeerId, Multiaddr),
) -> Result<Option<Orbit<BlockStores, BlockStage>>> {
    if !store_config.exists(&orbit).await? {
        return Ok(None);
    }
    load_orbit_inner(
        orbit,
        store_config.clone(),
        stage_config.clone(),
        db.to_string(),
        relay,
    )
    .await
    .map(Some)
}

// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
#[cached(size = 100, result = true, sync_writes = true)]
async fn load_orbit_inner(
    orbit: OrbitId,
    store_config: BlockConfig,
    staging: BlockStage,
    db: String,
    relay: (PeerId, Multiaddr),
) -> Result<Orbit<BlockStores, BlockStage>> {
    debug!("loading orbit {}", &orbit);
    Orbit::open(
        &OrbitPeerConfigBuilder::<BlockConfig, BlockStage>::default()
            .manifest(
                Manifest::resolve_dyn(&orbit, None)
                    .await?
                    .ok_or_else(|| anyhow!("Orbit DID Document not resolvable"))?,
            )
            .identity(
                // TODO allow using sql db as peer identity store
                store_config
                    .key_pair(&orbit)
                    .await?
                    .ok_or_else(|| anyhow!("Peer Identity key could not be found"))?,
            )
            .store(store_config)
            .staging(staging)
            .db(db)
            .relay(relay)
            .build()?,
    )
    .await?
    .ok_or_else(|| anyhow!("Orbit could not be opened: not found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{IndexStorage, LocalIndexStorage},
        BlockConfig, FileSystemConfig,
    };
    use kepler_lib::resolver::DID_METHODS;
    use kepler_lib::ssi::{
        did::{Source, DIDURL},
        jwk::JWK,
    };
    use std::convert::TryInto;
    use tempfile::{tempdir, TempDir};

    async fn op(md: Manifest) -> anyhow::Result<(Orbit<BlockStores>, TempDir)> {
        let dir = tempdir()?;
        Ok((
            Orbit::create(
                &OrbitPeerConfigBuilder::<BlockConfig, IndexStorage>::default()
                    .identity(Ed25519Keypair::generate())
                    .manifest(md)
                    .blocks(BlockConfig::B(FileSystemConfig::new(dir.path())))
                    .index(IndexStorage::Local(LocalIndexStorage {
                        path: dir.path().into(),
                    }))
                    .build()?,
            )
            .await?,
            dir,
        ))
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

        let (orbit, dir) = op(md).await.unwrap();
    }
}
