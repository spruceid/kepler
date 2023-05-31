use crate::{manifest::Manifest, BlockConfig, BlockStage, BlockStores};
use anyhow::{anyhow, Result};
use derive_builder::Builder;
use kepler_core::{
    hash::Hash,
    models::{kv_write::Metadata, *},
    sea_orm,
    storage::{Content, ImmutableReadStore, StorageConfig},
    OrbitDatabase,
};
use kepler_lib::resource::OrbitId;
use libp2p::{core::Multiaddr, identity::Keypair, PeerId};
use rocket::tokio::task::JoinHandle;

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
    identity: Keypair,
    #[builder(setter(into))]
    manifest: Manifest,
    #[builder(setter(into, strip_option), default)]
    relay: Option<(PeerId, Multiaddr)>,
    #[builder(setter(into))]
    store: B,
    #[builder(setter(into))]
    staging: S,
}

impl<B, S> Orbit<B, S> {
    async fn open<CB, CS>(
        config: &OrbitPeerConfig<CB, CS>,
        conn: sea_orm::DatabaseConnection,
    ) -> anyhow::Result<Option<Self>>
    where
        CB: StorageConfig<B>,
        CB::Error: 'static + Sync + Send,
        CS: StorageConfig<S>,
        CS::Error: 'static + Sync + Send,
    {
        let _local_peer_id = PeerId::from_public_key(&config.identity.public());
        let _relay = &config.relay;

        let store = match config.store.open(config.manifest.id()).await? {
            Some(b) => b,
            None => return Ok(None),
        };
        let staging = match config.staging.open(config.manifest.id()).await? {
            Some(b) => b,
            None => return Ok(None),
        };

        let capabilities = OrbitDatabase::wrap(conn, config.manifest.id().clone()).await?;

        Ok(Some(Orbit {
            manifest: config.manifest.clone(),
            capabilities,
            store,
            staging,
        }))
    }

    async fn create<CB, CS>(
        config: &OrbitPeerConfig<CB, CS>,
        conn: sea_orm::DatabaseConnection,
    ) -> anyhow::Result<Self>
    where
        CB: StorageConfig<B>,
        CB::Error: 'static + Sync + Send,
        CS: StorageConfig<S>,
        CS::Error: 'static + Sync + Send,
    {
        let _local_peer_id = config.identity.public().to_peer_id();
        let _relay = &config.relay;

        let store = config.store.create(config.manifest.id()).await?;
        let staging = config.staging.create(config.manifest.id()).await?;

        let capabilities = OrbitDatabase::wrap(conn, config.manifest.id().clone()).await?;

        Ok(Orbit {
            manifest: config.manifest.clone(),
            capabilities,
            store,
            staging,
        })
    }

    pub async fn list(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        use sea_orm::{entity::prelude::*, query::*};
        // get content id for key from db
        let mut list = kv_write::Entity::find()
            .filter(
                Condition::all()
                    .add(kv_write::Column::Key.starts_with(prefix))
                    .add(kv_write::Column::Orbit.eq(self.manifest.id().to_string())),
            )
            .order_by_desc(kv_write::Column::Seq)
            .order_by_desc(kv_write::Column::EpochId)
            .find_also_related(kv_delete::Entity)
            .filter(kv_delete::Column::InvocationId.is_null())
            .all(&self.capabilities.readable().await?)
            .await?
            .into_iter()
            .map(|(kv, _)| kv.key)
            .collect::<Vec<String>>();
        list.dedup();
        Ok(list)
    }

    pub async fn metadata(
        &self,
        key: &str,
        version: Option<(i64, Hash)>,
    ) -> anyhow::Result<Option<Metadata>> {
        match self.get_kv_entity(key, version).await? {
            Some(entry) => Ok(Some(entry.metadata)),
            None => Ok(None),
        }
    }

    async fn get_kv_entity(
        &self,
        key: &str,
        version: Option<(i64, Hash)>,
    ) -> Result<Option<kv_write::Model>, sea_orm::DbErr> {
        use sea_orm::{entity::prelude::*, query::*};
        Ok(if let Some((seq, epoch)) = version {
            kv_write::Entity::find_by_id((self.manifest.id().to_string(), seq, epoch))
                .find_also_related(kv_delete::Entity)
                .filter(kv_delete::Column::InvocationId.is_null())
                .one(&self.capabilities.readable().await?)
                .await?
                .map(|(kv, _)| kv)
        } else {
            kv_write::Entity::find()
                .filter(
                    Condition::all()
                        .add(kv_write::Column::Key.eq(key))
                        .add(kv_write::Column::Orbit.eq(self.manifest.id().to_string())),
                )
                .order_by_desc(kv_write::Column::Seq)
                .order_by_desc(kv_write::Column::EpochId)
                .find_also_related(kv_delete::Entity)
                .filter(kv_delete::Column::InvocationId.is_null())
                .one(&self.capabilities.readable().await?)
                .await?
                .map(|(kv, _)| kv)
        })
    }
}

impl<B, S> Orbit<B, S>
where
    B: ImmutableReadStore,
    B::Error: 'static,
{
    pub async fn get(
        &self,
        key: &str,
        version: Option<(i64, Hash)>,
    ) -> anyhow::Result<Option<(Content<B::Readable>, Metadata)>> {
        // get content id for key from db
        let entry = match self.get_kv_entity(key, version).await? {
            Some(entry) => entry,
            None => return Ok(None),
        };
        let content = match self.store.read(&entry.value).await? {
            Some(content) => content,
            None => return Err(anyhow!("content indexed but not found")),
        };
        Ok(Some((content, entry.metadata)))
    }
}

#[async_trait]
pub trait ProviderUtils {
    type Error: StdError;
    async fn exists(&self, orbit: &OrbitId) -> Result<bool, Self::Error>;
    async fn relay_key_pair(&self) -> Result<Keypair, Self::Error>;
    async fn key_pair(&self, orbit: &OrbitId) -> Result<Option<Keypair>, Self::Error>;
    async fn setup_orbit(&self, orbit: &OrbitId, key: &Keypair) -> Result<(), Self::Error>;
}

// Using Option to distinguish when the orbit already exists from a hard error
pub async fn create_orbit(
    id: &OrbitId,
    store_config: &BlockConfig,
    staging_config: &BlockStage,
    db: &sea_orm::DatabaseConnection,
    relay: (PeerId, Multiaddr),
    kp: Keypair,
) -> Result<Option<Orbit<BlockStores, BlockStage>>> {
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
        &OrbitPeerConfigBuilder::<BlockConfig, BlockStage>::default()
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
            .store(store_config.clone())
            .staging(staging_config.clone())
            .relay(relay.clone())
            .build()?,
        db.clone(),
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
    db: &sea_orm::DatabaseConnection,
    relay: (PeerId, Multiaddr),
) -> Result<Option<Orbit<BlockStores, BlockStage>>> {
    if !store_config.exists(&orbit).await? {
        return Ok(None);
    }
    load_orbit_inner(
        orbit,
        store_config.clone(),
        stage_config.clone(),
        db.clone(),
        relay,
    )
    .await
    .map(Some)
}

// Not using this function directly because cached cannot handle Result<Option<>> well.
// 100 orbits => 600 FDs
// #[cached(size = 100, result = true, sync_writes = true)]
async fn load_orbit_inner(
    orbit: OrbitId,
    store_config: BlockConfig,
    staging: BlockStage,
    db: sea_orm::DatabaseConnection,
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
            .relay(relay)
            .build()?,
        db,
    )
    .await?
    .ok_or_else(|| anyhow!("Orbit could not be opened: not found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockConfig, BlockStage, FileSystemConfig};
    use kepler_core::{sea_orm, storage::memory::MemoryStaging};
    use kepler_lib::resolver::DID_METHODS;
    use kepler_lib::ssi::{
        did::{Source, DIDURL},
        jwk::JWK,
    };
    use std::convert::TryInto;
    use tempfile::{tempdir, TempDir};

    async fn op(md: Manifest) -> anyhow::Result<(Orbit<BlockStores, BlockStage>, TempDir)> {
        let dir = tempdir()?;
        Ok((
            Orbit::create(
                &OrbitPeerConfigBuilder::<BlockConfig, BlockStage>::default()
                    .identity(Keypair::generate_ed25519())
                    .manifest(md)
                    .store(BlockConfig::B(FileSystemConfig::new(dir.path())))
                    .staging(BlockStage::B(MemoryStaging))
                    .build()?,
                sea_orm::Database::connect("sqlite::memory:").await?,
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

        let (_orbit, _dir) = op(md).await.unwrap();
    }
}
