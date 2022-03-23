use libipld::{cid::Cid, DagCbor};

pub struct Capabilities;

#[derive(DagCbor)]
pub struct Event {
    pub prev: Vec<Cid>,
    pub priority: u64,
    pub delegate: Vec<LinkedUpdate>
    pub revoke: Vec<LinkedUpdate>,
}

/// References a Policy Event and it's Parent LinkedUpdate
#[derive(DagCbor)]
pub struct LinkedUpdate {
    pub update: Cid,
    pub parent: Cid
}

#[derive(DagCbor)]
pub struct Invocations {
    pub prev: Vec<Cid>,
    pub invocations: Vec<LinkedUpdate>
}

#[derive(PartialEq)]
pub struct Delegation {
}

impl Delegation {
    pub fn id(&self) -> &str { "" };
    pub fn delegate(&self) -> &str { "" };
    pub fn delegator(&self) -> &str { "" };
    pub fn parent(&self) -> &str { "" };
    pub fn resources(&self) -> &[ResourceId] { &[] }
}

#[derive(PartialEq)]
pub struct Invocation;

impl Invocation {
    pub fn id(&self) -> &str { "" };
    pub fn invoker(&self) -> &str { "" };
    pub fn target(&self) -> &ResourceId;
    pub fn parent(&self) -> &str { "" };
}

#[derive(PartialEq)]
pub struct Revocation;

impl Revocation {
    pub fn id(&self) -> &str { "" };
    pub fn revoker(&self) -> &str { "" };
    pub fn revoked(&self) -> &str { "" };
}

#[derive(Default)]
pub struct Transaction {
    pub delegations: Vec<Delegation>,
    pub revocations: Vec<Revocation>
}

impl From<Delegation> for Transaction {
    fn from(d: Delegation) -> Self {
        Self::new([d], vec![])
    }
}

impl <I: IntoIterator<Item=Delegation>>From<I> for Transaction {
    fn from(d: I) -> Self {
        Self::new(d, vec![])
    }
}

impl From<Revocation> for Transaction {
    fn from(r: Revocation) -> Self {
        Self::new(vec![], [r])
    }
}

impl <I: IntoIterator<Item=Revocation>>From<I> for Transaction {
    fn from(r: I) -> Self {
        Self::new(vec![], r)
    }
}

impl Transaction {
    pub fn new<D, R>(d: D, r: R) -> Self where D: IntoIterator<Item=Delegation>, R: IntoIterator<Item=Revocation>{
        Self { delegations: d.into_iter().collect(), revocations: r.into_iter().collect() }
    }
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
    #[test]
    async fn invoke() {
        let caps = Capabilities;
        let inv = Invocation;

        let res = caps.invoke(vec![inv]).unwrap();
        assert_eq!(caps.get_invocation(inv.id()).unwrap().unwrap(), inv);
    }

    #[test]
    async fn delegate() {
        let caps = Capabilities;

        let del = Delegation;
        let del_res = caps.transact(del.into()).unwrap();
        assert_eq!(caps.get_delegation(del.id()).unwrap().unwrap(), del);

        let inv = Invocation;
        let inv_res = caps.invoke(vec![inv]).unwrap();
        assert_eq!(caps.get_invocation(inv.id()).unwrap().unwrap(), inv);
    }

    #[test]
    async fn revoke() {
        let caps = Capabilities;

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
        let caps = Capabilities;

        let dels = vec![Delegation, Delegation, Delegation];
        let del_res = caps.transact(dels.into()).unwrap();
        assert_eq!(caps.get_delegation(del.id()).unwrap().unwrap(), del);

        let delegated = caps.capabilities_for("").unwrap().unwrap();
        assert_eq!(dels, delegated);
    }
}
