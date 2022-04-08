use anyhow::Result;
use libipld::{cid::Cid, multibase::Base};
use std::convert::TryFrom;
use thiserror::Error;

use crate::{config, storage::KV};

#[derive(Clone)]
pub struct AddRemoveSetStore {
    elements: KV,
    tombs: KV,
}

#[derive(Error, Debug)]
pub enum Error<E> {
    #[error(transparent)]
    Store(#[from] sled::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    #[error(transparent)]
    ElementDeser(E),
}

impl AddRemoveSetStore {
    pub async fn new(
        orbit_id: Cid,
        subsystem_name: String,
        config: config::IndexStorage,
    ) -> Result<Self> {
        // map key to element cid
        let elements = KV::new(
            orbit_id,
            subsystem_name.clone(),
            "elements".to_string(),
            config.clone(),
        )
        .await?;
        // map key to element cid
        let tombs = KV::new(orbit_id, subsystem_name, "tombs".to_string(), config).await?;
        Ok(Self { elements, tombs })
    }
    pub async fn element<N: AsRef<[u8]>, E: TryFrom<Vec<u8>>>(
        &self,
        n: N,
    ) -> Result<Option<E>, Error<E::Error>> {
        self.elements
            .get(n.as_ref())
            .await?
            .map(|b| E::try_from(b.to_vec()).map_err(Error::ElementDeser))
            .transpose()
    }
    pub async fn elements<E: TryFrom<Vec<u8>>>(
        &self,
    ) -> Result<impl Iterator<Item = Result<(Vec<u8>, E), Error<E::Error>>>, anyhow::Error> {
        Ok(self.elements.elements().await?.into_iter().map(|r| {
            let (k, v) = r;
            let e = E::try_from(v).map_err(Error::ElementDeser)?;
            Ok((k, e))
        }))
    }
    pub async fn is_tombstoned<N: AsRef<[u8]>>(&self, n: N) -> Result<bool, anyhow::Error> {
        Ok(self.tombs.contains_key(n.as_ref()).await?)
    }
    pub async fn set_element<N: AsRef<[u8]>, E: AsRef<[u8]> + TryFrom<Vec<u8>>>(
        &self,
        n: N,
        e: &E,
    ) -> Result<Option<E>, Error<E::Error>> {
        self.elements
            .insert(n.as_ref(), e.as_ref())
            .await?
            .map(|b| E::try_from(b.to_vec()).map_err(Error::ElementDeser))
            .transpose()
    }
    pub async fn set_tombstone<N: AsRef<[u8]>>(&self, n: N) -> Result<(), anyhow::Error> {
        self.tombs.insert(n.as_ref(), &[]).await?;
        Ok(())
    }
}
