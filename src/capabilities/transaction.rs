#[derive(Default)]
pub struct Updates {
    pub delegations: Vec<Delegation>,
    pub revocations: Vec<Revocation>,
}

impl From<Delegation> for Updates {
    fn from(d: Delegation) -> Self {
        Self::new([d], vec![])
    }
}

impl<I: IntoIterator<Item = Delegation>> From<I> for Updates {
    fn from(d: I) -> Self {
        Self::new(d, vec![])
    }
}

impl From<Revocation> for Updates {
    fn from(r: Revocation) -> Self {
        Self::new(vec![], [r])
    }
}

impl<I: IntoIterator<Item = Revocation>> From<I> for Updates {
    fn from(r: I) -> Self {
        Self::new(vec![], r)
    }
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
