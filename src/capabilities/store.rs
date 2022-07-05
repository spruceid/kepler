use crate::{
    indexes::{AddRemoveSetStore, HeadStore},
    ipfs::{Block, Ipfs},
    kv::to_block_raw,
    zcap::{CapStore, Delegation, Invocation, Revocation, Verifiable},
};
use anyhow::Result;
use async_recursion::async_recursion;
use cacaos::siwe_cacao::SiweCacao;
use futures::stream::{self, TryStreamExt};
use kepler_lib::{
    resource::{OrbitId, ResourceId},
    zcap::{KeplerDelegation, KeplerInvocation},
};
use libipld::{
    cbor::{DagCbor, DagCborCodec},
    codec::{Decode, Encode},
    multibase::Base,
    multihash::Code,
    Cid, DagCbor,
};
use rocket::futures::future::try_join_all;

use crate::config;

#[derive(DagCbor, PartialEq, Debug, Clone)]
pub struct AuthRef(Cid, Vec<u8>);

impl AuthRef {
    pub fn new(event_cid: Cid, invocation_id: Vec<u8>) -> Self {
        Self(event_cid, invocation_id)
    }
}

#[derive(DagCbor, PartialEq, Debug, Clone)]
enum BlockRef {
    SameBlock,
    Block(Cid),
}

#[derive(DagCbor, PartialEq, Debug, Clone)]
pub(crate) struct ElementRef(BlockRef, Vec<u8>);

const SERVICE_NAME: &str = "capabilities";

#[derive(Clone)]
pub struct Store {
    pub id: ResourceId,
    pub ipfs: Ipfs,
    pub(crate) root: String,
    index: AddRemoveSetStore,
    delegation_heads: HeadStore,
    invocation_heads: HeadStore,
}

impl Store {
    pub async fn new(oid: &OrbitId, ipfs: Ipfs, config: config::IndexStorage) -> Result<Self> {
        let id = oid
            .clone()
            .to_resource(Some(SERVICE_NAME.to_string()), None, None);
        let oid_string = oid.to_string();
        let root = oid.did();
        let index =
            AddRemoveSetStore::new(oid.get_cid(), "capabilities".to_string(), config.clone())
                .await?;

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
            ipfs,
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
        self.broadcast_heads().await?;
        Ok(())
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

        let cid = self.ipfs.put_block(eb.to_block()?).await?;

        // write element indexes
        try_join_all(event.delegate.into_iter().map(|d| async {
            // add backlink in index
            self.index
                .set_element(&d.1.block.cid().to_bytes(), &d.0.block.cid().to_bytes())
                .await?;
            tracing::debug!("applied delegation {:?}", d.1.block.cid());
            // put delegation block (encoded ucan or cacao)
            self.ipfs.put_block(d.1.block).await?;
            // put link block
            self.ipfs.put_block(d.0.block).await?;
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
            self.ipfs.put_block(r.1.block).await?;
            // put link block
            self.ipfs.put_block(r.0.block).await?;
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
        let caps = crate::zcap::MultiCollection(event, self);
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

    pub async fn invoke(&self, invocations: impl IntoIterator<Item = Invocation>) -> Result<Cid> {
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
        self.broadcast_heads().await?;
        Ok(cid)
    }

    async fn get_invocation(&self, i: &Cid) -> Result<Invocations> {
        let update: InvocationsBlock = self.get_obj(i).await?.base;

        Ok(Invocations {
            prev: update.prev,
            invoke: try_join_all(update.invoke.iter().map(|c| async {
                let link: WithBlock<LinkedUpdate> = self.get_obj(c).await?;
                let inv: WithBlock<Invocation> = self.get_obj(&link.base.update).await?;
                Result::<(WithBlock<LinkedUpdate>, WithBlock<Invocation>)>::Ok((link, inv))
            }))
            .await?,
        })
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
        let cid = self.ipfs.put_block(eb.to_block()?).await?;

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

    async fn get_event(&self, e: &Cid) -> Result<Event> {
        let update: EventBlock = self.get_obj(e).await?.base;

        Ok(Event {
            prev: update.prev,
            delegate: try_join_all(update.delegate.iter().map(|c| async {
                let link: WithBlock<LinkedUpdate> = self.get_obj(c).await?;
                let del: WithBlock<Delegation> = self.get_obj(&link.base.update).await?;
                Result::<(WithBlock<LinkedUpdate>, WithBlock<Delegation>)>::Ok((link, del))
            }))
            .await?,
            revoke: try_join_all(update.revoke.iter().map(|c| async {
                let link: WithBlock<LinkedUpdate> = self.get_obj(c).await?;
                let rev: WithBlock<Revocation> = self.get_obj(&link.base.update).await?;
                Result::<(WithBlock<LinkedUpdate>, WithBlock<Revocation>)>::Ok((link, rev))
            }))
            .await?,
        })
    }

    async fn get_obj<T>(&self, c: &Cid) -> Result<WithBlock<T>>
    where
        T: FromBlock,
    {
        Ok(WithBlock::<T>::try_from(self.ipfs.get_block(&c).await?)?)
    }

    // async fn broadcast_update_verbatim(&self, event: Event) -> Result<()> {
    //     debug!("broadcasting update on {}", self.id);
    //     self.ipfs
    //         .pubsub_publish(
    //             self.id
    //                 .clone()
    //                 .get_cid()
    //                 .to_string_of_base(Base::Base58Btc)?,
    //             CapsMessage::Update(event).to_block()?.into_inner().1,
    //         )
    //         .await?;
    //     Ok(())
    // }

    pub(crate) async fn broadcast_heads(&self) -> Result<()> {
        let updates = self.delegation_heads.get_heads().await?.0;
        let invocations = self.invocation_heads.get_heads().await?.0;
        if !updates.is_empty() || !invocations.is_empty() {
            debug!(
                "broadcasting {} update heads and {} invocation heads on {}",
                updates.len(),
                invocations.len(),
                self.id,
            );
            self.ipfs
                .pubsub_publish(
                    self.id
                        .clone()
                        .get_cid()
                        .to_string_of_base(Base::Base58Btc)?,
                    CapsMessage::Heads {
                        updates,
                        invocations,
                    }
                    .to_block()?
                    .into_inner()
                    .1,
                )
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn request_heads(&self) -> Result<()> {
        self.ipfs
            .pubsub_publish(
                self.id
                    .clone()
                    .get_cid()
                    .to_string_of_base(Base::Base58Btc)?,
                CapsMessage::StateReq.to_block()?.into_inner().1,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn try_merge_heads(
        &self,
        updates: impl Iterator<Item = Cid> + Send,
        invocations: impl Iterator<Item = Cid> + Send,
    ) -> Result<()> {
        self.try_merge_updates(updates).await?;
        self.try_merge_invocations(invocations).await?;
        Ok(())
    }

    #[async_recursion]
    async fn try_merge_updates(
        &self,
        updates: impl Iterator<Item = Cid> + Send + 'async_recursion,
    ) -> Result<()> {
        try_join_all(updates.map(|head| async move {
            if self.delegation_heads.get_height(&head).await?.is_some() {
                return Ok(());
            };
            let update = self.get_event(&head).await?;

            self.try_merge_updates(
                stream::iter(update.prev.iter().map(Ok).collect::<Vec<Result<_>>>())
                    .try_filter_map(|d| async move {
                        self.delegation_heads.get_height(d).await.map(|o| match o {
                            Some(_) => None,
                            None => Some(*d),
                        })
                    })
                    .try_collect::<Vec<Cid>>()
                    .await?
                    .into_iter(),
            )
            .await?;

            self.apply(update).await
        }))
        .await?;
        Ok(())
    }

    #[async_recursion]
    pub(crate) async fn try_merge_invocations(
        &self,
        invocations: impl Iterator<Item = Cid> + Send + 'async_recursion,
    ) -> Result<()> {
        try_join_all(invocations.map(|head| async move {
            if self.invocation_heads.get_height(&head).await?.is_some() {
                return Result::<()>::Ok(());
            };

            let invs: Invocations = self.get_invocation(&head).await?;

            self.try_merge_invocations(
                stream::iter(invs.prev.iter().map(Ok).collect::<Vec<Result<_>>>())
                    .try_filter_map(|i| async move {
                        self.invocation_heads.get_height(i).await.map(|o| match o {
                            Some(_) => None,
                            None => Some(*i),
                        })
                    })
                    .try_collect::<Vec<Cid>>()
                    .await?
                    .into_iter(),
            )
            .await?;

            self.apply_invocations(invs).await?;
            Ok(())
        }))
        .await?;
        Ok(())
    }
}

#[rocket::async_trait]
impl CapStore for Store {
    async fn get_cap(&self, c: &Cid) -> Result<Option<Delegation>> {
        // annoyingly ipfs will error if it cant find something, so we probably dont want to error here
        Ok(self.get_obj(c).await.ok().map(|d| d.base))
    }
}

#[derive(Debug, Clone)]
struct WithBlock<T> {
    pub block: Block,
    pub base: T,
}

impl<T> WithBlock<T>
where
    T: ToBlock,
{
    pub fn new(base: T) -> Result<Self> {
        Ok(Self {
            base,
            block: base.to_block()?,
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
            block,
            base: T::from_block(&block)?,
        })
    }
}

#[derive(DagCbor, Clone, Debug)]
pub(crate) enum CapsMessage {
    Invocation(Cid),
    Update(EventBlock),
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
                Ok(block.decode()?)
            }
        }
    };
}

impl_toblock!(CapsMessage);
impl_toblock!(SiweCacao);

impl ToBlock for KeplerInvocation {
    fn to_block(&self) -> Result<Block> {
        Ok(self.to_block(Code::Blake3_256)?)
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

impl FromBlock for KeplerInvocation {
    fn from_block(block: &Block) -> Result<Self> {
        Ok(KeplerInvocation::from_block(block)?)
    }
}

impl FromBlock for KeplerDelegation {
    fn from_block(block: &Block) -> Result<Self> {
        if block.codec() == u64::from(DagCborCodec) {
            Ok(Self::Cacao(block.decode()?))
        } else {
            Ok(Self::Ucan(kepler_lib::ssi::ucan::Ucan::from_block(block)?))
        }
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
        Ok(self
            .delegate
            .iter()
            .find_map(|d| {
                if d.1.block.cid() == c {
                    Some(d.1.base.clone())
                } else {
                    None
                }
            })
            .clone())
    }
}

#[derive(DagCbor, Debug, Clone)]
struct EventBlock {
    pub prev: Vec<Cid>,
    pub delegate: Vec<Cid>,
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
mod test {
    // use super::*;
    // use crate::ipfs::create_ipfs;
    // use ipfs::Keypair;
    // async fn get_store(id: &OrbitId) -> Store {
    //     let tmp = tempdir::TempDir::new("test_streams").unwrap();
    //     let kp = Keypair::generate_ed25519();
    //     let (ipfs, ipfs_task, receiver) = create_ipfs(id.to_string(), &tmp.path(), kp, [])
    //         .await
    //         .unwrap();
    //     let db = sled::open(tmp.path().join("db.sled")).unwrap();
    //     tokio::spawn(ipfs_task);
    //     Store::new(id, ipfs, &db).unwrap()
    // }
    // fn orbit() -> OrbitId {
    //     "kepler:did:example:123://orbit0".parse().unwrap()
    // }
    // fn invoke(id: &[u8], target: ResourceId, parent: &[u8], invoker: &[u8]) -> Invocation {
    //     Invocation {
    //         id: id.into(),
    //         parent: parent.into(),
    //         target,
    //         invoker: invoker.into(),
    //         message: vec![],
    //     }
    // }
    // #[test]
    // async fn simple_invoke() {
    //     let oid = orbit();
    //     let caps = get_store(&oid).await;
    //     let inv = invoke(
    //         "inv0".as_bytes(),
    //         oid.clone()
    //             .to_resource(Some("kv".into()), Some("images".into()), None),
    //         oid.to_string().as_bytes(),
    //         "invoker1".as_bytes(),
    //     );
    //     let res = caps.invoke(vec![inv]).await.unwrap();
    //     assert_eq!(caps.get_invocation(&inv.id()).await.unwrap().unwrap(), inv);
    // }

    // #[test]
    // async fn delegate() {
    //     let caps = get_store().await;

    //     let del = Delegation;
    //     let del_res = caps.transact(del.into()).await.unwrap();
    //     assert_eq!(caps.get_delegation(&del.id()).await.unwrap().unwrap(), del);

    //     let inv = Invocation;
    //     let inv_res = caps.invoke(vec![inv]).unwrap();
    //     assert_eq!(caps.get_invocation(inv.id()).unwrap().unwrap(), inv);
    // }

    // #[test]
    // async fn revoke() {
    //     let caps = get_store();

    //     let del = Delegation;
    //     let del_res = caps.transact(del.into()).unwrap();
    //     assert_eq!(caps.get_delegation(del.id()).unwrap().unwrap(), del);

    //     let inv = Invocation;
    //     let inv_res = caps.invoke(vec![inv]).unwrap();
    //     assert_eq!(caps.get_invocation(inv.id()).unwrap().unwrap(), inv);

    //     let rev = Revocation;
    //     let rev_res = caps.transact(rev.into()).unwrap();
    //     assert_eq!(caps.get_revocation(rev.id()).unwrap().unwrap(), rev);

    //     let inv2 = Invocation;
    //     let inv_res2 = caps.invoke(vec![inv2]);

    //     assert!(inv_res2.is_err());
    //     assert_eq!(caps.get_invocation(inv2.id()).unwrap(), None);
    // }

    // #[test]
    // async fn get_caps() {
    //     let caps = get_store();

    //     let dels = vec![Delegation, Delegation, Delegation];
    //     let del_res = caps.transact(dels.into()).unwrap();
    //     assert_eq!(caps.get_delegation(del.id()).unwrap().unwrap(), del);

    //     let delegated = caps.capabilities_for("").unwrap().unwrap();
    //     assert_eq!(dels, delegated);
    // }
}
