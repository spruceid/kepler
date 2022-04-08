use sled::{Db, Tree};
use std::convert::TryFrom;

use thiserror::Error;

#[derive(Clone)]
pub struct AddRemoveSetStore {
    elements: Tree,
    tombs: Tree,
}

#[derive(Error, Debug)]
pub enum Error<E> {
    #[error(transparent)]
    Store(#[from] sled::Error),
    #[error(transparent)]
    ElementDeser(E),
}

impl AddRemoveSetStore {
    pub fn new(db: &Db, id: &[u8]) -> Result<Self, sled::Error> {
        // map key to element cid
        let elements = db.open_tree([id, ".elements".as_bytes()].concat())?;
        // map key to element cid
        let tombs = db.open_tree([id, ".tombs".as_bytes()].concat())?;
        Ok(Self { elements, tombs })
    }
    pub fn element<'a, N: AsRef<[u8]>, E: TryFrom<Vec<u8>>>(
        &self,
        n: N,
    ) -> Result<Option<E>, Error<E::Error>> {
        Ok(self
            .elements
            .get(n.as_ref())?
            .map(|b| E::try_from(b.to_vec()).map_err(Error::ElementDeser))
            .transpose()?)
    }
    pub fn elements<'a, E: TryFrom<Vec<u8>>>(
        &self,
    ) -> impl Iterator<Item = Result<(Vec<u8>, E), Error<E::Error>>> {
        self.elements.iter().map(|r| {
            let (k, v) = r?;
            let e = E::try_from(v.to_vec()).map_err(Error::ElementDeser)?;
            Ok((k.to_vec(), e))
        })
    }
    pub fn is_tombstoned<N: AsRef<[u8]>>(&self, n: N) -> Result<bool, sled::Error> {
        self.tombs.contains_key(n.as_ref())
    }
    pub fn set_element<'a, N: AsRef<[u8]>, E: AsRef<[u8]> + TryFrom<Vec<u8>>>(
        &self,
        n: N,
        e: &E,
    ) -> Result<Option<E>, Error<E::Error>> {
        Ok(self
            .elements
            .insert(n.as_ref(), e.as_ref())?
            .map(|b| E::try_from(b.to_vec()).map_err(Error::ElementDeser))
            .transpose()?)
    }
    pub fn set_tombstone<N: AsRef<[u8]>>(&self, n: N) -> Result<(), sled::Error> {
        self.tombs.insert(n.as_ref(), &[])?;
        Ok(())
    }
}
