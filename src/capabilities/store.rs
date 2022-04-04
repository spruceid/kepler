use crate::{
    heads::HeadStore,
    ipfs::{Block, Ipfs},
    resource::ResourceId,
    siwe::SIWEMessage,
    zcap::{KeplerDelegation, KeplerInvocation},
};
use anyhow::Result;
use libipld::{
    cbor::{DagCbor, DagCborCodec},
    multihash::Code,
    prelude::*,
    Cid, DagCbor,
};
use rocket::futures::future::{try_join_all, TryFutureExt};
use sled::{Db, Tree};
use ssi::vc::URI;
use std::convert::TryFrom;

#[derive(DagCbor)]
pub struct AuthRef(Cid, Vec<u8>);

#[derive(Clone)]
pub struct Store<H> {
    pub id: Vec<u8>,
    pub ipfs: Ipfs,
    elements: Tree,
    tombs: Tree,
    heads: H,
}

#[derive(Clone)]
pub struct Service<H> {
    pub store: Store<H>,
}

impl<H> std::ops::Deref for Service<H> {
    type Target = Store<H>;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl<H> Store<H> {
    pub fn new(id: Vec<u8>, ipfs: Ipfs, db: Db, heads: H) -> Result<Self> {
        // map key to element cid
        let elements = db.open_tree("elements")?;
        // map key to element cid
        let tombs = db.open_tree("tombs")?;
        Ok(Self {
            id,
            ipfs,
            elements,
            tombs,
            heads,
        })
    }
    pub fn is_revoked(&self, d: &[u8]) -> Result<Option<bool>> {
        Ok(
            match (self.elements.contains_key(d)?, self.is_tombstoned(d)?) {
                (false, false) => None,
                (_, true) => Some(true),
                (true, r) => Some(r),
            },
        )
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

    fn is_tombstoned(&self, id: &[u8]) -> Result<bool> {
        Ok(self.tombs.contains_key(id)?)
    }
    fn tombstone(&self, id: &[u8]) -> Result<()> {
        self.tombs.insert(id, &[])?;
        Ok(())
    }

    async fn element_decoded<T>(&self, id: &[u8]) -> Result<Option<T>>
    where
        T: Decode<DagCborCodec>,
    {
        Ok(self
            .element_block(id)
            .await?
            .map(|b| b.decode())
            .transpose()?)
    }
    async fn element_block(&self, id: &[u8]) -> Result<Option<Block>> {
        Ok(match self.element_cid(id)? {
            Some(c) => Some(self.ipfs.get_block(&c).await?),
            None => None,
        })
    }
    fn element_cid(&self, id: &[u8]) -> Result<Option<Cid>> {
        Ok(self
            .elements
            .get(id)?
            .map(|b| Cid::try_from(b.as_ref()))
            .transpose()?)
    }
    fn set_element(&self, id: &[u8], cid: &Cid) -> Result<()> {
        if !self.elements.contains_key(id)? {
            self.elements.insert(id, cid.to_bytes())?;
        }
        Ok(())
    }
    fn link_update<U>(&self, update: U) -> Result<LinkedUpdate<U>>
    where
        U: DagCbor + IndexReferences,
    {
        Ok(LinkedUpdate {
            parent: self
                .element_cid(update.parent())?
                .ok_or_else(|| anyhow!("unknown parent capability"))?,
            update,
        })
    }
}

impl<H> Store<H>
where
    H: HeadStore,
{
    pub async fn transact(&self, updates: Updates) -> Result<()> {
        self.apply(self.make_event(updates)?).await?;
        // TODO broadcast now
        //
        Ok(())
    }

    async fn apply(&self, event: Event) -> Result<()> {
        let block = event.to_block()?;

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
            self.set_element(e.update.id(), block.cid())?;
        }
        for e in event.revoke.iter() {
            // revoke
            self.set_element(e.update.id(), block.cid())?;
            self.tombstone(e.update.revoked())?;
        }

        // commit heads
        let (heads, _) = self.heads.get_heads()?;
        self.ipfs.put_block(block).await?;
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

    pub async fn invoke(&self, invocations: impl IntoIterator<Item = Invocation>) -> Result<()> {
        self.apply_invocations(Invocations {
            // TODO we need a separate heads tracker for invocations
            prev: todo!(),
            invoke: try_join_all(invocations.into_iter().map(|i| async move {
                i.verify().await?;
                self.link_update(i)
            }))
            .await?,
        })
        .await?;
        // TODO broadcast now
        //
        Ok(())
    }

    async fn apply_invocations(&self, event: Invocations) -> Result<()> {
        let cid = self.ipfs.put_block(event.to_block()?).await?;

        for e in event.invoke.iter() {
            self.set_element(e.update.id(), &cid)?;
        }

        // TODO commit heads
        //
        Ok(())
    }

    fn make_event(
        &self,
        Updates {
            delegations,
            revocations,
        }: Updates,
    ) -> Result<Event> {
        Ok(Event {
            prev: self.heads.get_heads()?.0,
            revoke: revocations
                .into_iter()
                .map(|r| self.link_update(r))
                .collect::<Result<Vec<LinkedUpdate<Revocation>>>>()?,
            delegate: delegations
                .into_iter()
                .map(|d| self.link_update(d))
                .collect::<Result<Vec<LinkedUpdate<Delegation>>>>()?,
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
        Ok(Block::encode(DagCborCodec, Code::Blake3_256, self)?)
    }
}

#[derive(DagCbor)]
struct Event {
    pub prev: Vec<Cid>,
    pub delegate: Vec<LinkedUpdate<Delegation>>,
    pub revoke: Vec<LinkedUpdate<Revocation>>,
}

/// References a Policy Event and it's Parent LinkedUpdate
#[derive(DagCbor)]
struct LinkedUpdate<U>
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

#[derive(DagCbor)]
struct Invocations {
    pub prev: Vec<Cid>,
    pub invoke: Vec<LinkedUpdate<Invocation>>,
}

#[derive(PartialEq, DagCbor)]
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
    fn try_from(d: (Vec<u8>, SIWEMessage)) -> Result<Self, Self::Error> {
        Ok(Self {
            message: serde_json::to_vec(&d.1)?,
            // TODO calculate ID
            id: d.1.nonce.into(),
            parent: d.0,
            resources: d
                .1
                .resources
                .into_iter()
                .map(|u| {
                    u.as_str()
                        .parse()
                        .map_err(|_| DelegationConversionError::MissingResources)
                })
                .collect::<Result<Vec<ResourceId>, Self::Error>>()?,
            delegator: d.1.address.into(),
            delegate: d.1.uri.as_str().into(),
        })
    }
}

#[rocket::async_trait]
impl EndVerifiable for Delegation {
    async fn verify(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(PartialEq, DagCbor)]
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
        Ok(Self {
            message: serde_json::to_vec(&i)?,
            // TODO verify ID binding if we have
            id: match i.id {
                URI::String(s) => s.into(),
            },
            parent: i
                .proof
                .and_then(|p| p.property_set.as_ref())
                .and_then(|ps| ps.get("capability").cloned())
                .and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.into()),
                    _ => None,
                })
                .ok_or(InvocationConversionError::MissingParent)?,
            target: i.property_set.invocation_target,
            invoker: i
                .proof
                .and_then(|p| p.verification_method)
                .ok_or(InvocationConversionError::MissingInvoker)?
                .into(),
        })
    }
}

impl TryFrom<(Vec<u8>, SIWEMessage)> for Invocation {
    type Error = InvocationConversionError<serde_json::Error>;
    fn try_from(i: (Vec<u8>, SIWEMessage)) -> Result<Self, Self::Error> {
        Ok(Self {
            message: serde_json::to_vec(&i.1)?,
            // TODO calculate ID
            id: i.1.nonce.into(),
            parent: i.0,
            target: i
                .1
                .uri
                .as_str()
                .parse()
                .map_err(|_| Self::Error::MissingResource)?,
            invoker: i.1.address.into(),
        })
    }
}

#[rocket::async_trait]
impl EndVerifiable for Invocation {
    async fn verify(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(PartialEq, DagCbor)]
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
        _ => return Err(RevocationConversionError::InvalidTarget),
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
    use super::*;
    use crate::heads::SledHeadStore;
    fn get_store() -> Store<SledHeadStore> {
        todo!()
    }
    #[test]
    async fn invoke() {
        let caps = get_store();
        let inv = Invocation;

        let res = caps.invoke(vec![inv]).unwrap();
        assert_eq!(caps.get_invocation(&inv.id()).await.unwrap().unwrap(), inv);
    }

    #[test]
    async fn delegate() {
        let caps = get_store();

        let del = Delegation;
        let del_res = caps.transact(del.into()).await.unwrap();
        assert_eq!(caps.get_delegation(&del.id()).await.unwrap().unwrap(), del);

        let inv = Invocation;
        let inv_res = caps.invoke(vec![inv]).unwrap();
        assert_eq!(caps.get_invocation(inv.id()).unwrap().unwrap(), inv);
    }

    #[test]
    async fn revoke() {
        let caps = get_store();

        let del = Delegation;
        let del_res = caps.transact(del.into()).unwrap();
        assert_eq!(caps.get_delegation(del.id()).unwrap().unwrap(), del);

        let inv = Invocation;
        let inv_res = caps.invoke(vec![inv]).unwrap();
        assert_eq!(caps.get_invocation(inv.id()).unwrap().unwrap(), inv);

        let rev = Revocation;
        let rev_res = caps.transact(rev.into()).unwrap();
        assert_eq!(caps.get_revocation(rev.id()).unwrap().unwrap(), rev);

        let inv2 = Invocation;
        let inv_res2 = caps.invoke(vec![inv2]);

        assert!(inv_res2.is_err());
        assert_eq!(caps.get_invocation(inv2.id()).unwrap(), None);
    }

    #[test]
    async fn get_caps() {
        let caps = get_store();

        let dels = vec![Delegation, Delegation, Delegation];
        let del_res = caps.transact(dels.into()).unwrap();
        assert_eq!(caps.get_delegation(del.id()).unwrap().unwrap(), del);

        let delegated = caps.capabilities_for("").unwrap().unwrap();
        assert_eq!(dels, delegated);
    }
}
