use crate::{
    indexes::{AddRemoveSetStore, HeadStore},
    ipfs::{Block, Ipfs},
    kv::to_block_raw,
    resource::{OrbitId, ResourceId},
    zcap::{CapNode, Delegation, Invocation, Revocation, Verifiable},
};
use anyhow::Result;
use async_recursion::async_recursion;
use futures::stream::{self, TryStreamExt};
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
    pub(crate) root: Vec<u8>,
    index: AddRemoveSetStore,
    delegation_heads: HeadStore,
    invocation_heads: HeadStore,
}

pub(crate) fn encode_root(s: &str) -> String {
    use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
    // Emulate encodeURIComponent
    const CHARS: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'<')
        .add(b'>')
        .add(b'`')
        .add(b':')
        .add(b'/');
    let target_encoded = utf8_percent_encode(s, CHARS);
    format!("urn:zcap:root:{}", target_encoded)
}

pub(crate) fn decode_root(s: &str) -> Result<OrbitId> {
    use percent_encoding::percent_decode_str;
    let r: ResourceId = percent_decode_str(s)
        .decode_utf8()?
        .strip_prefix("urn:zcap:root:")
        .ok_or_else(|| anyhow!("Invalid Root Zcap Prefix"))?
        .parse()?;
    Ok(r.orbit().clone())
}

impl Store {
    pub async fn new(oid: &OrbitId, ipfs: Ipfs, config: config::IndexStorage) -> Result<Self> {
        let id = oid
            .clone()
            .to_resource(Some(SERVICE_NAME.to_string()), None, None);
        let oid_string = oid.to_string();
        let root = encode_root(&oid_string);
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
            root: root.into_bytes(),
        })
    }
    pub async fn is_revoked(&self, d: &[u8]) -> Result<bool> {
        self.index.is_tombstoned(d).await
    }
    pub async fn get_delegation(&self, id: &[u8]) -> Result<Option<Delegation>> {
        Ok(self.element_decoded::<Event>(id).await?.and_then(|e| {
            e.delegate.into_iter().find_map(|d| {
                if id == d.update.id() {
                    Some(d.update)
                } else {
                    None
                }
            })
        }))
    }
    pub async fn get_invocation(&self, id: &[u8]) -> Result<Option<Invocation>> {
        Ok(self
            .element_decoded::<Invocations>(id)
            .await?
            .and_then(|e| {
                e.invoke.into_iter().find_map(|d| {
                    if id == d.update.id() {
                        Some(d.update)
                    } else {
                        None
                    }
                })
            }))
    }
    pub async fn get_revocation(&self, id: &[u8]) -> Result<Option<Revocation>> {
        Ok(self.element_decoded::<Event>(id).await?.and_then(|e| {
            e.revoke.into_iter().find_map(|r| {
                if id == r.update.id() {
                    Some(r.update)
                } else {
                    None
                }
            })
        }))
    }

    async fn element_decoded<T>(&self, id: &[u8]) -> Result<Option<T>>
    where
        T: Decode<DagCborCodec>,
    {
        self.element_block(id)
            .await?
            .map(|b| b.decode())
            .transpose()
    }
    async fn element_block(&self, id: &[u8]) -> Result<Option<Block>> {
        Ok(match self.index.element(id).await? {
            Some(c) => Some(self.ipfs.get_block(&c).await?),
            None => None,
        })
    }
    async fn link_update<U>(&self, update: U) -> Result<LinkedUpdate<U>>
    where
        U: DagCbor + CapNode,
    {
        Ok(LinkedUpdate {
            parents: try_join_all(update.parent_ids().map(|p| async move {
                Ok(ElementRef(
                    self.index
                        .element(&p)
                        .await?
                        .map(|c: Cid| BlockRef::Block(c))
                        .unwrap_or(BlockRef::SameBlock),
                    p.to_vec(),
                )) as Result<ElementRef, crate::indexes::Error<libipld::cid::Error>>
            }))
            .await?,
            update,
        })
    }

    pub async fn transact(&self, updates: Updates) -> Result<()> {
        let event = self.make_event(updates).await?;
        self.apply(&event).await?;
        self.broadcast_update_verbatim(event).await?;
        Ok(())
    }

    pub(crate) async fn apply(&self, event: &Event) -> Result<()> {
        let block = event.to_block()?;
        let cid = self.ipfs.put_block(block).await?;

        // verify everything
        self.verify(&event).await?;

        // write element indexes
        for e in event.delegate.iter() {
            // delegate
            // TODO for now, there is no conflict resolution policy for the mapping of
            // doc uuid => event cid, a partitioned peer may process a different doc
            // with the same uuid, associating the uuid with a different cid.
            // when the partition heals, they will have diverging capability indexes.
            // If we embed or commit to the cid in the uuid, we will never have this conflict
            self.index
                .set_element(e.update.id(), &cid.to_bytes())
                .await?;
            tracing::debug!("applied delegation {:?}", e.update.id());
        }
        for e in event.revoke.iter() {
            // revoke
            self.index.set_tombstone(e.update.revoked()).await?;
            self.index
                .set_element(e.update.id(), &cid.to_bytes())
                .await?;
        }

        // commit heads
        let (heads, h) = self.delegation_heads.get_heads().await?;
        self.delegation_heads.set_heights([(cid, h + 1)]).await?;
        self.delegation_heads.new_heads([cid], heads).await?;
        Ok(())
    }

    async fn verify(&self, event: &Event) -> Result<()> {
        // TODO ensure all uris extend parent uris
        try_join_all(event.delegate.iter().map(|u| self.verify_single(&u.update))).await?;
        // TODO ensure revocation permission (issuer or delegator)
        try_join_all(event.revoke.iter().map(|u| self.verify_single(&u.update))).await?;
        Ok(())
    }

    async fn verify_single<D: CapNode + Verifiable>(&self, d: &D) -> Result<()> {
        tracing::debug!("{:?}", d.root());
        match d.root() {
            Some(r) if r.as_bytes() == self.root => d.verify(None).await,
            _ => Err(anyhow!("Incorrect Root Capability")),
        }
    }

    pub async fn invoke(&self, invocations: impl IntoIterator<Item = Invocation>) -> Result<Cid> {
        let (dels, invs): (Vec<Vec<Delegation>>, Vec<Invocation>) =
            try_join_all(invocations.into_iter().map(|i| async move {
                i.verify(None).await?;
                Ok((self.explode_unseen_parents(&i).await?, i))
                    as Result<(Vec<Delegation>, Invocation)>
            }))
            .await?
            .into_iter()
            .unzip();
        self.transact(Updates::new(dels.into_iter().flatten(), []))
            .await?;
        let cid = self
            .apply_invocations(Invocations {
                prev: self.invocation_heads.get_heads().await?.0,
                invoke: try_join_all(invs.into_iter().map(|i| async move {
                    tracing::debug!("invoking {:?}", i.parent_ids().next().unwrap_or(vec![]));
                    self.link_update(i).await
                }))
                .await?,
            })
            .await?;
        self.broadcast_heads().await?;
        Ok(cid)
    }

    pub(crate) async fn apply_invocations(&self, event: Invocations) -> Result<Cid> {
        let cid = self.ipfs.put_block(event.to_block()?).await?;

        for e in event.invoke.iter() {
            self.index
                .set_element(e.update.id(), &cid.to_bytes())
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
        // for all delegations:
        // 1. extract unseen parents into top levels
        let rd = try_join_all(revocations.iter().map(|r| self.explode_unseen_parents(r))).await?;
        let dd = try_join_all(delegations.iter().map(|d| self.explode_unseen_parents(d))).await?;
        // 2. link all events
        Ok(Event {
            prev: self.delegation_heads.get_heads().await?.0,
            revoke: stream::iter(
                revocations
                    .into_iter()
                    .map(Ok)
                    .collect::<Vec<Result<Revocation>>>(),
            )
            .and_then(|r| async { self.link_update(r).await })
            .try_collect()
            .await?,
            delegate: stream::iter(
                delegations
                    .into_iter()
                    .chain(rd.into_iter().flatten())
                    .chain(dd.into_iter().flatten())
                    .map(Ok)
                    .collect::<Vec<Result<Delegation>>>(),
            )
            .and_then(|d| async { self.link_update(d).await })
            .try_collect()
            .await?,
        })
    }

    async fn explode_unseen_parents<C: CapNode>(&self, node: &C) -> Result<Vec<Delegation>> {
        let mut parents = vec![];
        for p in node.parents() {
            match self.index.element::<&[u8], Cid>(&p.id()).await? {
                Some(_) => break,
                None => parents.push(p),
            }
        }
        Ok(parents)
    }

    async fn broadcast_update_verbatim(&self, event: Event) -> Result<()> {
        debug!("broadcasting update on {}", self.id);
        self.ipfs
            .pubsub_publish(
                self.id
                    .clone()
                    .get_cid()
                    .to_string_of_base(Base::Base58Btc)?,
                CapsMessage::Update(event).to_block()?.into_inner().1,
            )
            .await?;
        Ok(())
    }

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
            let update_block = self.ipfs.get_block(&head).await?;
            let update: Event = update_block.decode()?;

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

            self.apply(&update).await
        }))
        .await?;
        Ok(())
    }

    #[async_recursion]
    async fn try_merge_invocations(
        &self,
        invocations: impl Iterator<Item = Cid> + Send + 'async_recursion,
    ) -> Result<()> {
        try_join_all(invocations.map(|head| async move {
            let invocation_block = self.ipfs.get_block(&head).await?;
            let invs: Invocations = invocation_block.decode()?;

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

            self.apply_invocations(invs).await
        }))
        .await?;
        Ok(())
    }
}

#[derive(DagCbor, Clone, Debug)]
pub(crate) enum CapsMessage {
    Invocation(Cid),
    Update(Event),
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

trait ToBlock {
    fn to_block(&self) -> Result<Block>;
}

impl<T> ToBlock for T
where
    T: Encode<DagCborCodec>,
{
    fn to_block(&self) -> Result<Block> {
        Block::encode(DagCborCodec, Code::Blake3_256, self)
    }
}

#[derive(DagCbor, Debug, Clone)]
pub(crate) struct Event {
    pub prev: Vec<Cid>,
    pub delegate: Vec<LinkedUpdate<Delegation>>,
    pub revoke: Vec<LinkedUpdate<Revocation>>,
}

/// References a Policy Event and it's Parent LinkedUpdate
#[derive(DagCbor, Debug, Clone)]
pub(crate) struct LinkedUpdate<U>
where
    U: DagCbor,
{
    pub update: U,
    pub parents: Vec<ElementRef>,
}

#[derive(DagCbor, Debug)]
pub(crate) struct Invocations {
    pub prev: Vec<Cid>,
    pub invoke: Vec<LinkedUpdate<Invocation>>,
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
