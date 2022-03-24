use crate::ipfs::Ipfs;
use anyhow::Result;
use libipld::{cbor::DagCborCodec, cid::Cid, codec::Decode};
use sled::{Batch, Db, Tree};
use std::convert::TryInto;

#[derive(Clone)]
pub struct SledHeadStore {
    heights: Tree,
    heads: Tree,
}

pub struct Heads<S> {
    store: S,
    ipfs: Ipfs,
}

pub trait Head: Decode<DagCborCodec> {
    fn priority(&self) -> u64;
    fn previous_heads(&self) -> &[Cid];
}

pub struct Merge {
    new: Vec<Cid>,
    obsoleted: Vec<Cid>,
}

impl<S: HeadStore> Heads<S> {
    async fn get_height<H: Head>(&self, c: &Cid) -> Result<Option<u64>> {
        Ok(match self.store.get_height(c)? {
            Some(h) => Some(h),
            None => {
                let head: H = self.ipfs.get_block(c).await?.decode()?;
                Some(head.priority())
            }
        })
    }
    pub async fn greater_than<H1: Head, H2: Head>(&self, a: &Cid, b: &Cid) -> Result<Option<bool>> {
        Ok(
            match (
                self.get_height::<H1>(a).await?,
                self.get_height::<H2>(b).await?,
            ) {
                (Some(ah), Some(bh)) if ah != bh => Some(ah > bh),
                (Some(_), Some(_)) => Some(a > b),
                _ => None,
            },
        )
    }
    pub fn set_heights(&self, heights: impl IntoIterator<Item = (Cid, u64)>) -> Result<()> {
        self.store.set_heights(heights)
    }
}

pub trait HeadStore {
    fn get_height(&self, c: &Cid) -> Result<Option<u64>>;
    fn get_heads(&self) -> Result<(Vec<Cid>, u64)>;
    fn set_heights(&self, heights: impl IntoIterator<Item = (Cid, u64)>) -> Result<()>;
    fn new_heads(
        &self,
        new_heads: impl IntoIterator<Item = Cid>,
        removed_heads: impl IntoIterator<Item = Cid>,
    ) -> Result<()>;
}

impl SledHeadStore {
    pub fn new(db: &Db) -> Result<Self> {
        Ok(Self {
            heights: db.open_tree("heights")?,
            heads: db.open_tree("heads")?,
        })
    }
}

impl HeadStore for SledHeadStore {
    fn get_heads(&self) -> Result<(Vec<Cid>, u64)> {
        self.heads.iter().try_fold(
            (vec![], 0),
            |(mut heads, max_height), r| -> Result<(Vec<Cid>, u64)> {
                let (head, _) = r?;
                let height = v2u64(
                    self.heights
                        .get(&head)?
                        .ok_or_else(|| anyhow!("Failed to find head height"))?,
                )?;
                heads.push(head[..].try_into()?);
                Ok((heads, u64::max(max_height, height)))
            },
        )
    }

    fn get_height(&self, c: &Cid) -> Result<Option<u64>> {
        self.heights.get(c.to_bytes())?.map(v2u64).transpose()
    }

    fn set_heights(&self, heights: impl IntoIterator<Item = (Cid, u64)>) -> Result<()> {
        let mut batch = Batch::default();
        for (op, height) in heights.into_iter() {
            if self.get_height(&op)?.map(|h| height > h).unwrap_or(true) {
                debug!("setting head height {} {}", op, height);
                batch.insert(op.to_bytes(), &u642v(height));
            }
        }
        self.heights.apply_batch(batch)?;
        Ok(())
    }

    fn new_heads(
        &self,
        new_heads: impl IntoIterator<Item = Cid>,
        removed_heads: impl IntoIterator<Item = Cid>,
    ) -> Result<()> {
        let mut batch = Batch::default();
        for n in new_heads {
            batch.insert(n.to_bytes(), &[]);
        }
        for r in removed_heads {
            batch.remove(r.to_bytes());
        }
        self.heads.apply_batch(batch)?;
        Ok(())
    }
}

pub(crate) fn v2u64<V: AsRef<[u8]>>(v: V) -> Result<u64> {
    Ok(u64::from_be_bytes(v.as_ref().try_into()?))
}

pub(crate) fn u642v(n: u64) -> [u8; 8] {
    n.to_be_bytes()
}
