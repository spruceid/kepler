use anyhow::Result;
use libipld::cid::Cid;
use sled::{Batch, Db, Tree};
use std::convert::TryInto;

#[derive(Clone)]
pub struct HeadStore {
    heights: Tree,
    heads: Tree,
}

impl HeadStore {
    pub fn new<N: AsRef<[u8]>>(db: &Db, name: N) -> Result<Self> {
        Ok(Self {
            heights: db.open_tree([name.as_ref(), ".heights".as_bytes()].concat())?,
            heads: db.open_tree([name.as_ref(), ".heads".as_bytes()].concat())?,
        })
    }
    pub fn get_heads(&self) -> Result<(Vec<Cid>, u64)> {
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

    pub fn get_height(&self, c: &Cid) -> Result<Option<u64>> {
        self.heights.get(c.to_bytes())?.map(v2u64).transpose()
    }

    pub fn set_heights(&self, heights: impl IntoIterator<Item = (Cid, u64)>) -> Result<()> {
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

    pub fn new_heads(
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
