use crate::ipfs::Ipfs;
use anyhow::Result;
use libipld::{cid::Cid, DagCbor};
use sled::{Db, IVec, Tree};

pub struct Store<H> {
    pub id: String,
    pub ipfs: Ipfs,
    elements: Tree,
    tombs: Tree,
    heads: H,
}

impl<H> Store<H> {
    pub fn new(id: String, ipfs: Ipfs, db: Db, heads: H) -> Result<Self> {
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
    pub fn is_revoked(&self, d: &Cid) -> Result<Option<bool>> {
        Ok(
            match (
                self.elements.contains_key(d.hash().digest())?,
                self.tombs.contains_key(d.hash().digest())?,
            ) {
                (false, false) => None,
                (_, true) => Some(true),
                (true, r) => Some(r),
            },
        )
    }
    pub async fn get_delegation(&self, d: &Cid) -> Result<Option<Delegation>> {
        self.ipfs.get_block(d).await?.decode().map(Some)
    }
    pub async fn get_invocation(&self, i: &Cid) -> Result<Option<Invocation>> {
        self.ipfs.get_block(i).await?.decode().map(Some)
    }
    pub async fn get_revocation(&self, r: &Cid) -> Result<Option<Revocation>> {
        self.ipfs.get_block(r).await?.decode().map(Some)
    }
    pub async fn transact(&self, r: Transaction) -> Result<()> {

    }
}

#[derive(DagCbor)]
pub struct Event {
    pub prev: Vec<Cid>,
    pub priority: u64,
    pub delegate: Vec<LinkedUpdate>,
    pub revoke: Vec<LinkedUpdate>,
}

/// References a Policy Event and it's Parent LinkedUpdate
#[derive(DagCbor)]
pub struct LinkedUpdate {
    pub update: Cid,
    pub parent: Cid,
}

#[derive(DagCbor)]
pub struct Invocations {
    pub prev: Vec<Cid>,
    pub invocations: Vec<LinkedUpdate>,
}

#[derive(PartialEq, DagCbor)]
pub struct Delegation;

impl Delegation {
    pub fn id(&self) -> Cid {
        todo!()
    }
}

#[derive(PartialEq, DagCbor)]
pub struct Invocation;

impl Invocation {
    pub fn id(&self) -> Cid {
        todo!()
    }
}

#[derive(PartialEq, DagCbor)]
pub struct Revocation;

impl Revocation {
    pub fn id(&self) -> Cid {
        todo!()
    }
impl Capabilities {
    pub fn transact(&self, tx: Transaction) -> Result<(), ()> {}
    pub fn invoke(&self, invocations: Vec<Invocation>) -> Result<(), ()> {}

    pub fn capabilities_for(&self, did: &str) -> Result<Vec<Delegation>, ()> {}
    pub fn get_invocation(&self, id: &str) -> Result<Option<Invocation>, ()> {}
    pub fn get_delegation(&self, id: &str) -> Result<Option<Delegation>, ()> {}
    pub fn get_revocation(&self, id: &str) -> Result<Option<Revocation>, ()> {}
}

#[cfg(test)]
mod test {
    use crate::heads::SledHeadStore;
    fn get_store() -> Store<SledHeadStore> {
        todo!()
    }
    #[test]
    async fn invoke() {
        let caps = get_store();
        let inv = Invocation;

        let res = caps.invoke(vec![inv]).unwrap();
        assert_eq!(caps.get_invocation(inv.id()).unwrap().unwrap(), inv);
    }

    #[test]
    async fn delegate() {
        let caps = get_store();

        let del = Delegation;
        let del_res = caps.transact(del.into()).unwrap();
        assert_eq!(caps.get_delegation(del.id()).unwrap().unwrap(), del);

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
