use crate::{
    authorization::{CapStore, Delegation, Invocation, Revocation, Verifiable},
    indexes::{AddRemoveSetStore, HeadStore},
    kv::to_block_raw,
    storage::ImmutableStore,
    Block,
};
use anyhow::Result;
use async_stream::try_stream;
use kepler_lib::libipld::{cbor::DagCborCodec, multihash::Code, Cid, DagCbor};
use kepler_lib::{
    authorization::{KeplerDelegation, KeplerInvocation, KeplerRevocation, Query},
    cacaos::siwe_cacao::SiweCacao,
    resource::{OrbitId, ResourceId},
};
use rocket::futures::{
    future::try_join_all,
    stream::{futures_unordered::FuturesUnordered, Stream, StreamExt, TryStreamExt},
};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

use crate::config;

#[derive(Error, Debug)]
pub enum InvokeError {
    #[error(transparent)]
    Unauthorized(anyhow::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

const SERVICE_NAME: &str = "capabilities";

#[derive(Clone)]
pub struct Store<B> {
    pub id: ResourceId,
    pub(crate) root: String,
    blocks: B,
    index: AddRemoveSetStore,
    delegation_heads: HeadStore,
    invocation_heads: HeadStore,
}

impl<B> Store<B>
where
    B: ImmutableStore,
    B::Error: 'static,
{
    pub async fn new(oid: &OrbitId, blocks: B, config: config::IndexStorage) -> Result<Self> {
        let id = oid
            .clone()
            .to_resource(Some(SERVICE_NAME.to_string()), None, None);
        let root = oid.did();
        let index =
            AddRemoveSetStore::new(oid.get_cid(), SERVICE_NAME.to_string(), config.clone()).await?;

        let (cid, n) = to_block_raw(&root)?.into_inner();

        if index.element(&n).await? != Some(cid) {
            index.set_element(&n, &cid.to_bytes()).await?;
        };
        // heads tracking for delegations
        let delegation_heads = HeadStore::new(
            oid.get_cid(),
            "capabilities".to_string(),
            "delegations".to_string(),
            config.clone(),
        )
        .await?;
        // heads tracking for invocations
        let invocation_heads = HeadStore::new(
            oid.get_cid(),
            "capabilities".to_string(),
            "invocations".to_string(),
            config.clone(),
        )
        .await?;
        Ok(Self {
            id,
            blocks,
            index,
            delegation_heads,
            invocation_heads,
            root,
        })
    }
    pub async fn is_revoked(&self, d: &[u8]) -> Result<bool> {
        self.index.is_tombstoned(d).await
    }

    pub async fn transact(&self, updates: Updates) -> Result<()> {
        let event = self.make_event(updates).await?;
        self.apply(event).await?;
        Ok(())
    }

    pub async fn query(&self, query: Query, invoker: &str) -> Result<HashMap<Cid, Delegation>> {
        // traverse the graph collecting all delegations which fit the query
        let (cids, _h) = self.delegation_heads.get_heads().await?;

        match query {
            Query::All => Ok(self
                .traverse_delegations(&cids)
                .try_collect::<HashMap<Cid, Delegation>>()
                .await?),
        }
    }
    fn traverse_delegations(
        &self,
        starts: &[Cid],
    ) -> impl Stream<Item = Result<(Cid, Delegation)>> + '_ {
        self.traverse_events(starts)
            .map_ok(|eb| {
                // fetch delegations from each event
                let f = FuturesUnordered::new();
                for cid in eb.delegate.iter() {
                    f.push(self.get_obj::<LinkedUpdate>(*cid));
                }
                f
            })
            .try_flatten()
            .and_then(|linked_update| async {
                match linked_update {
                    Some(lu) => match self.get_obj(lu.base.update).await? {
                        Some(wb) => Ok((*wb.block.cid(), wb.base)),
                        None => Err(anyhow!("Could not find block")),
                    },
                    None => Err(anyhow!("Could not find block")),
                }
            })
    }
    fn traverse_events(&self, starts: &[Cid]) -> impl Stream<Item = Result<EventBlock>> + '_ {
        // get events
        // mark cid as traversed
        // get links
        // return Ok.chain(traverse(links)) ?
        let mut traversed: HashSet<Cid> = HashSet::new();
        let mut f = FuturesUnordered::new();
        for cid in starts.iter() {
            f.push(self.get_obj(*cid));
        }
        try_stream! {
            while let Some(r) = f.next().await {
                let wb = r?.ok_or_else(|| anyhow!("Coud not find block"))?;
                let eb: EventBlock = wb.base;
                let cid = wb.block.cid();
                if !traversed.contains(cid) {
                    traversed.insert(*cid);
                    for parent in eb.prev.iter() {
                        f.push(self.get_obj(*parent));
                    }
                    yield eb
                }
            }
        }
    }

    pub(crate) async fn apply(&self, event: Event) -> Result<()> {
        // given the event obj
        // verify everything
        self.verify(&event).await?;

        let eb = EventBlock {
            prev: event.prev,
            delegate: event.delegate.iter().map(|d| *d.0.block.cid()).collect(),
            revoke: event.revoke.iter().map(|r| *r.0.block.cid()).collect(),
        };
        let eb_block = eb.to_block()?;

        let cid = Cid::new_v1(
            eb_block.cid().codec(),
            self.blocks
                .write(eb_block.data(), eb_block.cid().hash().code().try_into()?)
                .await?,
        );

        // write element indexes
        try_join_all(event.delegate.into_iter().map(|d| async move {
            // add backlink in index
            self.index
                .set_element(&d.1.block.cid().to_bytes(), &d.0.block.cid().to_bytes())
                .await?;
            tracing::debug!("applied delegation {:?}", d.1.block.cid());
            // put delegation block (encoded ucan or cacao)
            self.blocks
                .write(d.1.block.data(), d.1.block.cid().hash().code().try_into()?)
                .await?;
            // put link block
            self.blocks
                .write(d.0.block.data(), d.0.block.cid().hash().code().try_into()?)
                .await?;
            Result::<()>::Ok(())
        }))
        .await?;
        try_join_all(event.revoke.into_iter().map(|r| async move {
            // revoke
            self.index
                .set_tombstone(&r.1.base.revoked.to_bytes())
                .await?;
            // add backlink in index
            self.index
                .set_element(&r.1.block.cid().to_bytes(), &r.0.block.cid().to_bytes())
                .await?;
            tracing::debug!("applied revocation {:?}", r.1.block.cid());
            // put revocation block (encoded ucan revocation or cacao)
            self.blocks
                .write(r.1.block.data(), r.1.block.cid().hash().code().try_into()?)
                .await?;
            // put link block
            self.blocks
                .write(r.0.block.data(), r.0.block.cid().hash().code().try_into()?)
                .await?;
            Result::<()>::Ok(())
        }))
        .await?;

        // commit heads
        let (heads, h) = self.delegation_heads.get_heads().await?;
        self.delegation_heads.set_heights([(cid, h + 1)]).await?;
        // should this be eb.prev instead of heads?
        self.delegation_heads.new_heads([cid], heads).await?;
        Ok(())
    }

    async fn verify(&self, event: &Event) -> Result<()> {
        // this allows us to verify using delegations present in the event but not in the store
        let caps = crate::authorization::MultiCollection(event, self);
        try_join_all(
            event
                .delegate
                .iter()
                .map(|d| async { d.1.base.verify(&caps, None, &self.root).await }),
        )
        .await?;
        try_join_all(
            event
                .revoke
                .iter()
                .map(|r| async { r.1.base.verify(&caps, None, &self.root).await }),
        )
        .await?;
        Ok(())
    }

    pub async fn invoke(
        &self,
        invocations: impl IntoIterator<Item = Invocation>,
    ) -> Result<Cid, InvokeError> {
        let cid = self
            .apply_invocations(Invocations {
                prev: self.invocation_heads.get_heads().await?.0,
                invoke: invocations
                    .into_iter()
                    .map(|i| {
                        let inv = WithBlock::new(i)?;
                        let link = WithBlock::new(LinkedUpdate {
                            update: *inv.block.cid(),
                            parents: inv.base.parents.clone(),
                        })?;
                        Result::<(WithBlock<LinkedUpdate>, WithBlock<Invocation>)>::Ok((link, inv))
                    })
                    .collect::<Result<Vec<(WithBlock<LinkedUpdate>, WithBlock<Invocation>)>>>()?,
            })
            .await?;
        Ok(cid)
    }
    pub(crate) async fn apply_invocations(&self, event: Invocations) -> Result<Cid> {
        try_join_all(
            event
                .invoke
                .iter()
                .map(|i| async { i.1.base.verify(self, None, &self.root).await }),
        )
        .await?;

        let eb = InvocationsBlock {
            prev: event.prev,
            invoke: event.invoke.iter().map(|i| *i.0.block.cid()).collect(),
        };
        let eb_block = eb.to_block()?;
        let cid = Cid::new_v1(
            eb_block.cid().codec(),
            self.blocks
                .write(eb_block.data(), eb_block.cid().hash().code().try_into()?)
                .await?,
        );

        for e in event.invoke.iter() {
            self.index
                .set_element(&e.1.block.cid().to_bytes(), &e.0.block.cid().to_bytes())
                .await?;
        }

        let (heads, h) = self.delegation_heads.get_heads().await?;
        self.invocation_heads.set_heights([(cid, h + 1)]).await?;
        self.invocation_heads.new_heads([cid], heads).await?;
        Ok(cid)
    }

    async fn make_event(
        &self,
        Updates {
            delegations,
            revocations,
        }: Updates,
    ) -> Result<Event> {
        Ok(Event {
            prev: self.delegation_heads.get_heads().await?.0,
            revoke: revocations
                .into_iter()
                .map(|r| {
                    let rev = WithBlock::new(r)?;
                    let link = WithBlock::new(LinkedUpdate {
                        update: *rev.block.cid(),
                        parents: rev.base.parents.clone(),
                    })?;
                    Ok((link, rev))
                })
                .collect::<Result<Vec<(WithBlock<LinkedUpdate>, WithBlock<Revocation>)>>>()?,
            delegate: delegations
                .into_iter()
                .map(|d| {
                    let del = WithBlock::new(d)?;
                    let link = WithBlock::new(LinkedUpdate {
                        update: *del.block.cid(),
                        parents: del.base.parents.clone(),
                    })?;
                    Ok((link, del))
                })
                .collect::<Result<Vec<(WithBlock<LinkedUpdate>, WithBlock<Delegation>)>>>()?,
        })
    }

    async fn get_obj<T>(&self, c: Cid) -> Result<Option<WithBlock<T>>>
    where
        T: FromBlock,
    {
        self.blocks
            .read_to_vec(c.hash())
            .await?
            .map(|v| Block::new(c, v).and_then(WithBlock::try_from))
            .transpose()
    }
}

#[rocket::async_trait]
impl<B> CapStore for Store<B>
where
    B: ImmutableStore,
    B::Error: 'static,
{
    async fn get_cap(&self, c: &Cid) -> Result<Option<Delegation>> {
        // annoyingly ipfs will error if it cant find something, so we probably dont want to error here
        self.get_obj(*c).await.map(|o| o.map(|d| d.base))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WithBlock<T> {
    pub block: Block,
    pub base: T,
}

impl<T> WithBlock<T>
where
    T: ToBlock,
{
    pub fn new(base: T) -> Result<Self> {
        Ok(Self {
            block: base.to_block()?,
            base,
        })
    }
}

impl<T> TryFrom<Block> for WithBlock<T>
where
    T: FromBlock,
{
    type Error = anyhow::Error;
    fn try_from(block: Block) -> Result<Self, Self::Error> {
        Ok(Self {
            base: T::from_block(&block)?,
            block,
        })
    }
}

#[derive(DagCbor, Clone, Debug)]
pub(crate) enum CapsMessage {
    Invocation(Cid),
    StateReq,
    Heads {
        updates: Vec<Cid>,
        invocations: Vec<Cid>,
    },
}

#[derive(Default)]
pub struct Updates {
    pub delegations: Vec<Delegation>,
    pub revocations: Vec<Revocation>,
}

impl Updates {
    pub fn new<D, R>(d: D, r: R) -> Self
    where
        D: IntoIterator<Item = Delegation>,
        R: IntoIterator<Item = Revocation>,
    {
        Self {
            delegations: d.into_iter().collect(),
            revocations: r.into_iter().collect(),
        }
    }
}

pub(crate) trait ToBlock {
    fn to_block(&self) -> Result<Block>;
}

pub(crate) trait FromBlock {
    fn from_block(block: &Block) -> Result<Self>
    where
        Self: Sized;
}

macro_rules! impl_toblock {
    ($type:ident) => {
        impl ToBlock for $type {
            fn to_block(&self) -> Result<Block> {
                Block::encode(DagCborCodec, Code::Blake3_256, self)
            }
        }
    };
}

macro_rules! impl_fromblock {
    ($type:ident) => {
        impl FromBlock for $type {
            fn from_block(block: &Block) -> Result<Self>
            where
                Self: Sized,
            {
                block.decode()
            }
        }
    };
}

impl_toblock!(CapsMessage);
impl_toblock!(SiweCacao);

impl ToBlock for KeplerInvocation {
    fn to_block(&self) -> Result<Block> {
        self.to_block(Code::Blake3_256)
    }
}

impl ToBlock for KeplerDelegation {
    fn to_block(&self) -> Result<Block> {
        match self {
            Self::Ucan(u) => u.to_block(Code::Blake3_256),
            Self::Cacao(c) => c.to_block(),
        }
    }
}

impl ToBlock for KeplerRevocation {
    fn to_block(&self) -> Result<Block> {
        match self {
            Self::Cacao(c) => c.to_block(),
        }
    }
}

impl FromBlock for KeplerInvocation {
    fn from_block(block: &Block) -> Result<Self> {
        KeplerInvocation::from_block(block)
    }
}

impl FromBlock for KeplerDelegation {
    fn from_block(block: &Block) -> Result<Self> {
        if block.codec() == u64::from(DagCborCodec) {
            Ok(Self::Cacao(Box::new(block.decode()?)))
        } else {
            Ok(Self::Ucan(Box::new(
                kepler_lib::ssi::ucan::Ucan::from_block(block)?,
            )))
        }
    }
}

impl FromBlock for KeplerRevocation {
    fn from_block(block: &Block) -> Result<Self> {
        Ok(Self::Cacao(block.decode()?))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Event {
    pub prev: Vec<Cid>,
    pub delegate: Vec<(WithBlock<LinkedUpdate>, WithBlock<Delegation>)>,
    pub revoke: Vec<(WithBlock<LinkedUpdate>, WithBlock<Revocation>)>,
}

#[rocket::async_trait]
impl CapStore for Event {
    async fn get_cap(&self, c: &Cid) -> Result<Option<Delegation>> {
        Ok(self.delegate.iter().find_map(|d| {
            if d.1.block.cid() == c {
                Some(d.1.base.clone())
            } else {
                None
            }
        }))
    }
}

#[derive(DagCbor, Debug, Clone)]
pub(crate) struct EventBlock {
    // links to EventBlock blocks
    pub prev: Vec<Cid>,
    // links to LinkedUpdate blocks
    pub delegate: Vec<Cid>,
    // links to LinkedUpdate blocks
    pub revoke: Vec<Cid>,
}

impl_toblock!(EventBlock);
impl_fromblock!(EventBlock);

/// References a Policy Event and it's Parent LinkedUpdate
#[derive(DagCbor, Debug, Clone)]
pub(crate) struct LinkedUpdate {
    pub update: Cid,
    pub parents: Vec<Cid>,
}

impl_fromblock!(LinkedUpdate);
impl_toblock!(LinkedUpdate);

#[derive(DagCbor, Debug)]
struct InvocationsBlock {
    pub prev: Vec<Cid>,
    pub invoke: Vec<Cid>,
}

impl_fromblock!(InvocationsBlock);
impl_toblock!(InvocationsBlock);

#[derive(Debug, Clone)]
pub(crate) struct Invocations {
    pub prev: Vec<Cid>,
    pub invoke: Vec<(WithBlock<LinkedUpdate>, WithBlock<Invocation>)>,
}

#[cfg(test)]
mod test {}
