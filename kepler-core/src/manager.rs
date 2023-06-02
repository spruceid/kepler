use crate::{
    db::Commit,
    manifest::{Manifest, ResolutionError},
    migrations::Migrator,
    orbit::{InvocationOutcome, OrbitPeer},
    storage::{
        ImmutableDeleteStore, ImmutableReadStore, ImmutableStaging, ImmutableWriteStore,
        StorageConfig,
    },
};
use kepler_lib::resource::OrbitId;
use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr};
use sea_orm_migration::MigratorTrait;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct OrbitPeerManager<B, S> {
    store: B,
    staging: S,
    database: DatabaseConnection,
}

#[derive(Debug, thiserror::Error)]
pub enum InitError<B, S> {
    #[error("failed to initialize database: {0}")]
    Database(#[from] DbErr),
    #[error("failed to initialize storage")]
    Storage(B),
    #[error("failed to initialize staging")]
    Staging(S),
    #[error("failed to resolve manifest: {0}")]
    Manifest(#[from] ResolutionError),
    #[error("manifest doesnt exist")]
    ManifestMissing,
}

impl<B, S> OrbitPeerManager<B, S> {
    pub async fn new<C: Into<ConnectOptions>>(
        store: B,
        staging: S,
        db_info: C,
    ) -> Result<OrbitPeerManager<B, S>, DbErr> {
        let database = Database::connect(db_info).await?;
        Migrator::up(&database, None).await?;
        Ok(Self::wrap(store, staging, database))
    }

    fn wrap(store: B, staging: S, database: DatabaseConnection) -> Self {
        OrbitPeerManager {
            store,
            staging,
            database,
        }
    }
}

impl<B, S> OrbitPeerManager<B, S> {
    pub async fn open<BO, SO>(
        &self,
        orbit: OrbitId,
    ) -> Result<Option<OrbitPeer<BO, SO, DatabaseConnection>>, InitError<B::Error, S::Error>>
    where
        B: StorageConfig<BO>,
        S: StorageConfig<SO>,
    {
        let store = match self.store.open(&orbit).await.map_err(InitError::Storage)? {
            Some(b) => b,
            None => return Ok(None),
        };
        let staging = match self
            .staging
            .open(&orbit)
            .await
            .map_err(InitError::Staging)?
        {
            Some(b) => b,
            None => return Ok(None),
        };
        let manifest = match Manifest::resolve_dyn(&orbit, None).await? {
            Some(m) => m,
            None => return Ok(None),
        };
        Ok(Some(OrbitPeer::new(
            manifest,
            self.database.clone(),
            store,
            staging,
        )))
    }

    pub async fn open_ref<BO, SO>(
        &self,
        orbit: OrbitId,
    ) -> Result<Option<OrbitPeer<BO, SO, &DatabaseConnection>>, InitError<B::Error, S::Error>>
    where
        B: StorageConfig<BO>,
        S: StorageConfig<SO>,
    {
        let store = match self.store.open(&orbit).await.map_err(InitError::Storage)? {
            Some(b) => b,
            None => return Ok(None),
        };
        let staging = match self
            .staging
            .open(&orbit)
            .await
            .map_err(InitError::Staging)?
        {
            Some(b) => b,
            None => return Ok(None),
        };
        let manifest = match Manifest::resolve_dyn(&orbit, None).await? {
            Some(m) => m,
            None => return Ok(None),
        };
        Ok(Some(OrbitPeer::new(
            manifest,
            &self.database,
            store,
            staging,
        )))
    }

    pub async fn create<BO, SO>(
        &self,
        orbit: OrbitId,
    ) -> Result<OrbitPeer<BO, SO, DatabaseConnection>, InitError<B::Error, S::Error>>
    where
        B: StorageConfig<BO>,
        S: StorageConfig<SO>,
    {
        let manifest = Manifest::resolve_dyn(&orbit, None)
            .await?
            .ok_or(InitError::ManifestMissing)?;
        let store = self
            .store
            .create(&orbit)
            .await
            .map_err(InitError::Storage)?;
        let staging = self
            .staging
            .create(&orbit)
            .await
            .map_err(InitError::Staging)?;
        Ok(OrbitPeer::new(
            manifest,
            self.database.clone(),
            store,
            staging,
        ))
    }

    pub async fn create_ref<BO, SO>(
        &self,
        orbit: OrbitId,
    ) -> Result<OrbitPeer<BO, SO, &DatabaseConnection>, InitError<B::Error, S::Error>>
    where
        B: StorageConfig<BO>,
        S: StorageConfig<SO>,
    {
        let manifest = Manifest::resolve_dyn(&orbit, None)
            .await?
            .ok_or(InitError::ManifestMissing)?;
        let store = self
            .store
            .create(&orbit)
            .await
            .map_err(InitError::Storage)?;
        let staging = self
            .staging
            .create(&orbit)
            .await
            .map_err(InitError::Staging)?;
        Ok(OrbitPeer::new(manifest, &self.database, store, staging))
    }
}

impl<B, S> OrbitPeerManager<B, S> {
    pub async fn delegate(&self, delegation: String) -> Result<HashMap<OrbitId, Commit>, ()> {
        todo!()
    }

    pub async fn revoke(&self, revocation: String) -> Result<HashMap<OrbitId, Commit>, ()> {
        todo!()
    }

    pub async fn invoke<BO, SO>(
        &self,
        invocation: String,
    ) -> Result<HashMap<OrbitId, (Commit, InvocationOutcome<BO::Readable>)>, ()>
    where
        B: StorageConfig<BO>,
        S: StorageConfig<SO>,
        BO: ImmutableReadStore + ImmutableWriteStore<SO> + ImmutableDeleteStore,
        SO: ImmutableStaging,
        SO::Writable: 'static,
    {
        todo!()
    }
}
