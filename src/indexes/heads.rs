use anyhow::Result;
use futures::stream::{self, TryStreamExt};
use libipld::cid::Cid;
use std::convert::TryInto;

use crate::{config, storage::KV};

#[derive(Clone)]
pub struct HeadStore {
    heights: KV,
    heads: KV,
}

impl HeadStore {
    pub async fn new(
        orbit_id: Cid,
        subsystem_name: String,
        name: String,
        config: config::IndexStorage,
    ) -> Result<Self> {
        Ok(Self {
            heights: KV::new(
                orbit_id,
                subsystem_name.clone(),
                format!("{}-heights", name),
                config.clone(),
            )
            .await?,
            heads: KV::new(orbit_id, subsystem_name, format!("{}-heads", name), config).await?,
        })
    }
    pub async fn get_heads(&self) -> Result<(Vec<Cid>, u64)> {
        stream::iter(
            self.heads
                .elements()
                .await?
                .into_iter()
                .map(|e| Ok(e))
                .collect::<Vec<anyhow::Result<(Vec<u8>, Vec<u8>)>>>(),
        )
        .try_fold(
            (vec![], 0),
            |(mut heads, max_height): (Vec<Cid>, u64), r: (Vec<u8>, Vec<u8>)| async move {
                let (head, _) = r;
                let height = v2u64(
                    self.heights
                        .get(&head)
                        .await?
                        .ok_or_else(|| anyhow!("Failed to find head height"))?,
                )?;
                heads.push(head[..].try_into()?);
                Ok((heads, u64::max(max_height, height)))
            },
        )
        // .collect()
        .await
    }

    pub async fn get_height(&self, c: &Cid) -> Result<Option<u64>> {
        self.heights.get(c.to_bytes()).await?.map(v2u64).transpose()
    }

    pub async fn set_heights(&self, heights: impl IntoIterator<Item = (Cid, u64)>) -> Result<()> {
        let mut batch = vec![];
        for (op, height) in heights.into_iter() {
            if self
                .get_height(&op)
                .await?
                .map(|h| height > h)
                .unwrap_or(true)
            {
                debug!("setting head height {} {}", op, height);
                batch.push((op.to_bytes(), u642v(height).to_vec()));
            }
        }
        self.heights.insert_batch(batch).await?;
        Ok(())
    }

    pub async fn new_heads(
        &self,
        new_heads: impl IntoIterator<Item = Cid>,
        removed_heads: impl IntoIterator<Item = Cid>,
    ) -> Result<()> {
        let mut batch_insert = vec![];
        let mut batch_remove = vec![];
        for n in new_heads {
            batch_insert.push((n.to_bytes(), vec![]));
        }
        for r in removed_heads {
            batch_remove.push(r.to_bytes());
        }
        self.heads.insert_batch(batch_insert).await?;
        self.heads.remove_batch(batch_remove).await?;
        Ok(())
    }
}

pub(crate) fn v2u64<V: AsRef<[u8]>>(v: V) -> Result<u64> {
    Ok(u64::from_be_bytes(v.as_ref().try_into()?))
}

pub(crate) fn u642v(n: u64) -> [u8; 8] {
    n.to_be_bytes()
}
