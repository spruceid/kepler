use crate::{
    db::{Commit, OrbitDatabase, TxError},
    events::*,
    hash::Hash,
    manifest::Manifest,
    models::{kv_write::Metadata, *},
    storage::{
        Content, ImmutableDeleteStore, ImmutableReadStore, ImmutableStaging, ImmutableWriteStore,
    },
    util::{DelegationError, DelegationInfo, InvocationInfo, RevocationInfo},
};
use futures::io::AsyncRead;
use kepler_lib::authorization::{
    EncodingError, HeaderEncode, KeplerDelegation, KeplerInvocation, KeplerRevocation,
};
use sea_orm::{DbErr, TransactionTrait};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct OrbitPeer<B, S, C> {
    manifest: Manifest,
    capabilities: OrbitDatabase<C>,
    store: B,
    staging: S,
}

impl<B, S, C> OrbitPeer<B, S, C> {
    pub(crate) fn new(manifest: Manifest, conn: C, store: B, staging: S) -> Self {
        Self {
            manifest,
            capabilities: OrbitDatabase::wrap(conn, manifest.id().clone()),
            store,
            staging,
        }
    }
}

impl<B, S, C> OrbitPeer<B, S, C>
where
    C: TransactionTrait,
{
    pub async fn delegate(&self, delegation: String) -> Result<Commit, TxError> {
        todo!()
    }

    pub async fn revoke(&self, revocation: String) -> Result<Commit, TxError> {
        todo!()
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>, DbErr> {
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

    async fn metadata(
        &self,
        key: &str,
        version: Option<(i64, Hash)>,
    ) -> Result<Option<Metadata>, DbErr> {
        match self.get_kv_entity(key, version).await? {
            Some(entry) => Ok(Some(entry.metadata)),
            None => Ok(None),
        }
    }

    async fn get_kv_entity(
        &self,
        key: &str,
        version: Option<(i64, Hash)>,
    ) -> Result<Option<kv_write::Model>, DbErr> {
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

#[derive(Debug, thiserror::Error)]
pub enum GetError<B> {
    #[error("database error: {0}")]
    Database(#[from] DbErr),
    #[error("store error")]
    Store(B),
    #[error("content indexed but not found")]
    NotFound,
}

impl<B, S, C> OrbitPeer<B, S, C>
where
    C: TransactionTrait,
    B: ImmutableReadStore,
{
    async fn get(
        &self,
        key: &str,
        version: Option<(i64, Hash)>,
    ) -> Result<Option<(Content<B::Readable>, Metadata)>, GetError<B::Error>> {
        // get content id for key from db
        let entry = match self.get_kv_entity(key, version).await? {
            Some(entry) => entry,
            None => return Ok(None),
        };
        let content = match self
            .store
            .read(&entry.value)
            .await
            .map_err(GetError::Store)?
        {
            Some(content) => content,
            None => return Err(GetError::NotFound),
        };
        Ok(Some((content, entry.metadata)))
    }
}

pub enum InvocationOutcome<R> {
    KvList(Vec<String>),
    KvDelete,
    KvMetadata(Metadata),
    KvWrite,
    KvRead(Option<(Metadata, Content<R>)>),
    OpenSessions(HashMap<Hash, DelegationInfo>),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Tx(#[from] TxError),
    #[error(transparent)]
    Encoding(#[from] EncodingError),
}

impl<B, S, C> OrbitPeer<B, S, C>
where
    C: TransactionTrait,
    S: ImmutableStaging,
    S::Writable: 'static,
    B: ImmutableReadStore + ImmutableWriteStore<S> + ImmutableDeleteStore,
{
    pub async fn invoke<R: AsyncRead>(
        &self,
        invocation: String,
        data: Option<(Metadata, R)>,
    ) -> Result<(Commit, InvocationOutcome<B::Readable>), Error> {
        todo!()
    }
}

#[cfg(test)]
mod tests {}
