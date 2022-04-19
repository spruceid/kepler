use crate::{
    indexes::{AddRemoveSetStore, HeadStore},
    ipfs::{Block, Ipfs},
    kv::to_block_raw,
    resource::{OrbitId, ResourceId},
    siwe::SIWEMessage,
    zcap::{KeplerDelegation, KeplerInvocation},
};
use anyhow::Result;
use futures::stream::{self, TryStreamExt};
use libipld::{
    cbor::{DagCbor, DagCborCodec},
    codec::{Decode, Encode},
    multihash::Code,
    Cid, DagCbor,
};
use rocket::futures::future::try_join_all;
use ssi::vc::URI;
use std::convert::TryFrom;

use crate::config;

#[derive(DagCbor, PartialEq, Debug, Clone)]
pub struct AuthRef(Cid, Vec<u8>);

impl AuthRef {
    pub fn new(event_cid: Cid, invocation_id: Vec<u8>) -> Self {
        Self(event_cid, invocation_id)
    }
}

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

impl Store {
    pub async fn new(id: &OrbitId, ipfs: Ipfs, config: config::IndexStorage) -> Result<Self> {
        let id = id
            .clone()
            .to_resource(Some(SERVICE_NAME.to_string()), None, None);
        let root = id.to_string().into_bytes();
        let index =
            AddRemoveSetStore::new(id.get_cid(), "capabilities".to_string(), config.clone())
                .await?;

        let (cid, n) = to_block_raw(&root)?.into_inner();

        if index.element(&n).await? != Some(cid) {
            index.set_element(&n, &cid.to_bytes()).await?;
        };
        // heads tracking for delegations
        let delegation_heads = HeadStore::new(
            id.get_cid(),
            "capabilities".to_string(),
            "delegations".to_string(),
            config.clone(),
        )
        .await?;
        // heads tracking for invocations
        let invocation_heads = HeadStore::new(
            id.get_cid(),
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
        U: DagCbor + IndexReferences,
    {
        Ok(LinkedUpdate {
            parent: self
                .index
                .element(update.parent())
                .await?
                .ok_or_else(|| anyhow!("unknown parent capability"))?,
            update,
        })
    }

    pub async fn transact(&self, updates: Updates) -> Result<()> {
        self.apply(self.make_event(updates).await?).await?;
        // TODO broadcast now
        //
        Ok(())
    }

    pub(crate) async fn apply(&self, event: Event) -> Result<()> {
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
        // TODO recursively check embedded parent delegations to see if they are valid
        // and/or already indexed
        // TODO ensure all uris extend parent uris
        try_join_all(event.delegate.iter().map(|u| u.update.verify())).await?;
        // TODO ensure revocation permission (issuer or delegator)
        try_join_all(event.revoke.iter().map(|u| u.update.verify())).await?;
        Ok(())
    }

    pub async fn invoke(&self, invocations: impl IntoIterator<Item = Invocation>) -> Result<Cid> {
        let cid = self
            .apply_invocations(Invocations {
                prev: self.invocation_heads.get_heads().await?.0,
                invoke: try_join_all(invocations.into_iter().map(|i| async move {
                    i.verify().await?;
                    tracing::debug!("invoking {:?}", i.parent());
                    self.link_update(i).await
                }))
                .await?,
            })
            .await?;
        // TODO broadcast now
        //
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
                    .map(Ok)
                    .collect::<Vec<Result<Delegation>>>(),
            )
            .and_then(|d| async { self.link_update(d).await })
            .try_collect()
            .await?,
        })
    }
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
    pub parent: Cid,
}

pub trait IndexReferences {
    fn id(&self) -> &[u8];
    fn parent(&self) -> &[u8];
}

#[rocket::async_trait]
trait EndVerifiable {
    // NOTE this assumes that all parent delegations are embedded in the document
    async fn verify(&self) -> Result<()>;
}

#[derive(DagCbor, Debug)]
pub(crate) struct Invocations {
    pub prev: Vec<Cid>,
    pub invoke: Vec<LinkedUpdate<Invocation>>,
}

#[derive(PartialEq, DagCbor, Debug, Clone)]
pub struct Delegation {
    id: Vec<u8>,
    parent: Vec<u8>,
    resources: Vec<ResourceId>,
    delegator: Vec<u8>,
    delegate: Vec<u8>,
    message: Vec<u8>,
}

impl IndexReferences for Delegation {
    fn id(&self) -> &[u8] {
        &self.id
    }
    fn parent(&self) -> &[u8] {
        &self.parent
    }
}

impl Delegation {
    pub fn resources(&self) -> &[ResourceId] {
        &self.resources
    }
    pub fn delegator(&self) -> &[u8] {
        &self.delegator
    }
    pub fn delegate(&self) -> &[u8] {
        &self.delegate
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DelegationConversionError<S> {
    #[error("Missing Parent Delegation ID")]
    MissingParent,
    #[error("Missing Delegator ID")]
    MissingDelegator,
    #[error("Missing Delegate ID")]
    MissingDelegate,
    #[error("Missing Resources")]
    MissingResources,
    #[error("Delegation ID is invalid")]
    InvalidId,
    #[error("Missing Delegation ID")]
    MissingId,
    #[error(transparent)]
    MessageSerialisation(#[from] S),
}

impl TryFrom<KeplerDelegation> for Delegation {
    type Error = DelegationConversionError<serde_json::Error>;
    fn try_from(d: KeplerDelegation) -> Result<Self, Self::Error> {
        Ok(Self {
            message: serde_json::to_vec(&d)?,
            // TODO verify ID binding if we have
            id: match d.id {
                URI::String(s) => s.into(),
            },
            parent: match d.parent_capability {
                URI::String(s) => s.into(),
            },
            resources: d.property_set.capability_action,
            delegator: d
                .proof
                .and_then(|p| p.verification_method)
                .ok_or(DelegationConversionError::MissingDelegator)?
                .into(),
            delegate: match d
                .invoker
                .ok_or(DelegationConversionError::MissingDelegate)?
            {
                URI::String(s) => s.into(),
            },
        })
    }
}

// HACK have to inject parent delegation ID
impl TryFrom<(Vec<u8>, SIWEMessage)> for Delegation {
    type Error = DelegationConversionError<serde_json::Error>;
    fn try_from((parent_id, message): (Vec<u8>, SIWEMessage)) -> Result<Self, Self::Error> {
        Ok(Self {
            message: serde_json::to_vec(&message)?,
            // TODO calculate ID
            id: ["urn:siwe:kepler:", &message.0.nonce].concat().into_bytes(),
            parent: parent_id,
            resources: message
                .0
                .resources
                .iter()
                .map(|u| {
                    u.as_str()
                        .parse()
                        .map_err(|_| DelegationConversionError::MissingResources)
                })
                .collect::<Result<Vec<ResourceId>, Self::Error>>()?,
            delegator: message.0.address.into(),
            delegate: message.0.uri.as_str().into(),
        })
    }
}

#[rocket::async_trait]
impl EndVerifiable for Delegation {
    async fn verify(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(PartialEq, DagCbor, Debug, Clone)]
pub struct Invocation {
    id: Vec<u8>,
    parent: Vec<u8>,
    target: ResourceId,
    invoker: Vec<u8>,
    message: Vec<u8>,
}

impl IndexReferences for Invocation {
    fn id(&self) -> &[u8] {
        &self.id
    }
    fn parent(&self) -> &[u8] {
        &self.parent
    }
}

impl Invocation {
    pub fn resource(&self) -> &ResourceId {
        &self.target
    }
    pub fn invoker(&self) -> &[u8] {
        &self.invoker
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InvocationConversionError<S> {
    #[error("Missing Parent Delegation ID")]
    MissingParent,
    #[error("Missing Invoker ID")]
    MissingInvoker,
    #[error("Missing Target Resource")]
    MissingResource,
    #[error("Invocation ID is invalid")]
    InvalidId,
    #[error("Missing Invocation ID")]
    MissingId,
    #[error(transparent)]
    MessageSerialisation(#[from] S),
}

impl TryFrom<KeplerInvocation> for Invocation {
    type Error = InvocationConversionError<serde_json::Error>;
    fn try_from(i: KeplerInvocation) -> Result<Self, Self::Error> {
        let message = serde_json::to_vec(&i)?;
        match i.proof {
            Some(p) => Ok(Self {
                message,
                // TODO verify ID binding if we have
                id: match i.id {
                    URI::String(s) => s.into(),
                },
                parent: p
                    .property_set
                    .as_ref()
                    .and_then(|ps| ps.get("capability").cloned())
                    .and_then(|v| match v {
                        serde_json::Value::String(s) => Some(s.into()),
                        _ => None,
                    })
                    .ok_or(InvocationConversionError::MissingParent)?,
                target: i.property_set.invocation_target,
                invoker: p
                    .verification_method
                    .ok_or(InvocationConversionError::MissingInvoker)?
                    .into(),
            }),
            None => Err(InvocationConversionError::MissingInvoker),
        }
    }
}

impl TryFrom<(Vec<u8>, SIWEMessage)> for Invocation {
    type Error = InvocationConversionError<serde_json::Error>;
    fn try_from((parent_id, message): (Vec<u8>, SIWEMessage)) -> Result<Self, Self::Error> {
        Ok(Self {
            message: serde_json::to_vec(&message)?,
            // TODO calculate ID
            id: ["urn:siwe:kepler:", &message.0.nonce].concat().into_bytes(),
            parent: parent_id,
            target: message
                .0
                .resources
                .iter()
                .find_map(|r| r.as_str().parse().ok())
                .ok_or(Self::Error::MissingResource)?,
            invoker: message.0.address.into(),
        })
    }
}

#[rocket::async_trait]
impl EndVerifiable for Invocation {
    async fn verify(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(PartialEq, DagCbor, Debug, Clone)]
pub struct Revocation {
    id: Vec<u8>,
    parent: Vec<u8>,
    target: Vec<u8>,
    revoker: Vec<u8>,
    message: Vec<u8>,
}

impl IndexReferences for Revocation {
    fn id(&self) -> &[u8] {
        &self.id
    }
    fn parent(&self) -> &[u8] {
        &self.parent
    }
}

impl Revocation {
    pub fn revoked(&self) -> &[u8] {
        &self.target
    }
    pub fn revoker(&self) -> &[u8] {
        &self.revoker
    }
}

fn check_target_is_delegation<S>(
    target: &ResourceId,
) -> Result<Vec<u8>, RevocationConversionError<S>> {
    match (
        target.service(),
        target.path().unwrap_or("").strip_prefix("/delegations/"),
    ) {
        // TODO what exactly do we expect here
        (Some("capabilities"), Some(p)) => Ok(p.into()),
        _ => Err(RevocationConversionError::InvalidTarget),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RevocationConversionError<S> {
    #[error("Target does not identify revokable Delegation")]
    InvalidTarget,
    #[error(transparent)]
    InvalidInvocation(#[from] InvocationConversionError<S>),
}

impl TryFrom<KeplerInvocation> for Revocation {
    type Error = RevocationConversionError<serde_json::Error>;
    fn try_from(i: KeplerInvocation) -> Result<Self, Self::Error> {
        let Invocation {
            id,
            parent,
            target,
            invoker,
            message,
        } = Invocation::try_from(i)?;
        Ok(Self {
            id,
            parent,
            target: check_target_is_delegation(&target)?,
            revoker: invoker,
            message,
        })
    }
}

impl TryFrom<(Vec<u8>, SIWEMessage)> for Revocation {
    type Error = RevocationConversionError<serde_json::Error>;
    fn try_from(i: (Vec<u8>, SIWEMessage)) -> Result<Self, Self::Error> {
        let Invocation {
            id,
            parent,
            target,
            invoker,
            message,
        } = Invocation::try_from(i)?;
        Ok(Self {
            id,
            parent,
            target: check_target_is_delegation(&target)?,
            revoker: invoker,
            message,
        })
    }
}

#[rocket::async_trait]
impl EndVerifiable for Revocation {
    async fn verify(&self) -> Result<()> {
        Ok(())
    }
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
