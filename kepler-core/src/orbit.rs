use crate::{
    db::{get_kv_entity, Commit, OrbitDatabase, TxError},
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
use sea_orm::{ConnectionTrait, DbErr, TransactionTrait};
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
            capabilities: OrbitDatabase::wrap(conn, manifest.id().clone()),
            manifest,
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
pub enum Error<T: std::error::Error> {
    #[error(transparent)]
    Tx(#[from] TxError),
    #[error(transparent)]
    Encoding(#[from] EncodingError),
    #[error(transparent)]
    TryInto(T),
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
        let (i, ser) = <KeplerInvocation as HeaderEncode>::decode(&invocation)?;
        let invocation = InvocationInfo::try_from(i)?;
        let res: ResourceId = invocation.capability.resource.parse()?;

        if let Some((metadata, data)) = data {
            let mut stage = self.staging.stage().await?;
        } else {
        }
        todo!()
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GetError<B> {
    #[error("database error: {0}")]
    Database(#[from] DbErr),
    #[error("store error")]
    Store(B),
    #[error("content indexed but not found")]
    NotFound,
}

pub(crate) async fn get<C, S>(
    db: &C,
    store: &S,
    orbit: &str,
    key: &str,
    version: Option<(i64, Hash)>,
) -> Result<Option<(Content<S::Readable>, Metadata)>, GetError<S::Error>>
where
    C: ConnectionTrait,
    S: ImmutableReadStore,
{
    // get content id for key from db
    let entry = match get_kv_entity(db, orbit, key, version).await? {
        Some(entry) => entry,
        None => return Ok(None),
    };
    let content = match store.read(&entry.value).await.map_err(GetError::Store)? {
        Some(content) => content,
        None => return Err(GetError::NotFound),
    };
    Ok(Some((content, entry.metadata)))
}

#[cfg(test)]
mod tests {}
