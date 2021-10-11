use crate::s3::{Object, ObjectBuilder, Service};
use anyhow::Result;
use async_recursion::async_recursion;
use libipld::{cid::Cid, DagCbor};
use rocket::futures::future::try_join_all;
use sled::{Batch, Db, Tree};
use std::convert::{TryFrom, TryInto};
use tracing::{debug, error};

use super::{to_block, to_block_raw, Block, Ipfs, KVMessage};

#[derive(DagCbor)]
struct Delta {
    // max depth
    pub priority: u64,
    pub add: Vec<Cid>,
    pub rmv: Vec<Cid>,
}

impl Delta {
    pub fn new(priority: u64, add: Vec<Cid>, rmv: Vec<Cid>) -> Self {
        Self { priority, add, rmv }
    }

    pub fn _merge(self, other: Self) -> Self {
        let mut add = self.add;
        let mut other_add = other.add;
        add.append(&mut other_add);

        let mut rmv = self.rmv;
        let mut other_rmv = other.rmv;
        rmv.append(&mut other_rmv);

        Self {
            add,
            rmv,
            priority: u64::max(self.priority, other.priority),
        }
    }
}

#[derive(DagCbor)]
struct LinkedDelta {
    // previous heads
    pub prev: Vec<Cid>,
    pub delta: Delta,
}

impl LinkedDelta {
    pub fn new(prev: Vec<Cid>, delta: Delta) -> Self {
        Self { prev, delta }
    }

    pub fn to_block(&self) -> Result<Block> {
        to_block(self)
    }
}

#[derive(Clone)]
pub struct Store {
    pub id: String,
    pub ipfs: Ipfs,
    elements: Tree,
    tombs: Tree,
    priorities: Tree,
    heads: Heads,
}

impl Store {
    pub fn new(id: String, ipfs: Ipfs, db: Db) -> Result<Self> {
        // map key to element cid
        let elements = db.open_tree("elements")?;
        // map key to element cid
        let tombs = db.open_tree("tombs")?;
        // map key to current max priority for key
        let priorities = db.open_tree("priorities")?;
        // map current DAG head cids to their priority
        let heads = Heads::new(db)?;
        Ok(Self {
            id,
            ipfs,
            elements,
            tombs,
            priorities,
            heads,
        })
    }
    pub fn get<N: AsRef<[u8]>>(&self, name: N) -> Result<Option<Object>> {
        let key = name;
        match self
            .elements
            .get(&key)?
            .map(|b| Cid::try_from(b.as_ref()))
            .transpose()?
        {
            Some(cid) => {
                if !self
                    .tombs
                    .contains_key([key.as_ref(), &cid.to_bytes()].concat())?
                {
                    Ok(Some(self.ipfs.get(&cid)?.decode()?))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    pub fn write(
        &self,
        // tuples of (obj-data, content bytes)
        add: impl IntoIterator<Item = (ObjectBuilder, Vec<u8>)>,
        // tuples of (key, opt (priority, obj-cid))
        remove: impl IntoIterator<Item = (Vec<u8>, Option<(u64, Cid)>)>,
    ) -> Result<()> {
        let (heads, height) = self.heads.state()?;
        let height = if heads.is_empty() && height == 0 {
            0
        } else {
            height + 1
        };
        // get s3 objects, s3 object blocks and data blocks to add
        let adds: Vec<(Object, Block, Block)> = add
            .into_iter()
            .map(|(s, v)| {
                let data_block = to_block_raw(&v)?;
                let s3_obj = s.add_content(*data_block.cid())?;
                let s3_block = s3_obj.to_block()?;
                Ok((s3_obj, s3_block, data_block))
            })
            .collect::<Result<Vec<(Object, Block, Block)>>>()?;
        let rmvs: Vec<(Vec<u8>, Cid)> = remove
            .into_iter()
            .map(|(key, version)| {
                Ok(match version {
                    Some((_, cid)) => {
                        // TODO check position better
                        (key, cid)
                    }
                    None => {
                        let cid = self
                            .elements
                            .get(&key)?
                            .map(|b| Cid::try_from(b.as_ref()))
                            .transpose()?
                            .ok_or(anyhow!("Failed to find Object ID for key {:?}", key))?;
                        (key, cid)
                    }
                })
            })
            .collect::<Result<Vec<(Vec<u8>, Cid)>>>()?;
        let delta = LinkedDelta::new(
            heads,
            Delta::new(
                height,
                adds.iter().map(|(_, b, _)| *b.cid()).collect(),
                rmvs.iter().map(|(_, c)| *c).collect(),
            ),
        );
        let block = delta.to_block()?;
        // apply/pin root/update heads
        self.apply(
            &(block, delta),
            adds.iter()
                .map(|(obj, block, _)| (*block.cid(), obj.clone()))
                .collect::<Vec<(Cid, Object)>>(),
            rmvs,
        )?;

        // insert children
        for (_, obj, data) in adds.iter() {
            self.ipfs.insert(obj)?;
            self.ipfs.insert(data)?;
        }
        // broadcast
        self.broadcast_heads()?;
        Ok(())
    }

    pub(crate) fn broadcast_heads(&self) -> Result<()> {
        let (heads, height) = self.heads.state()?;
        if !heads.is_empty() {
            debug!("broadcasting {} heads at maxheight {}", heads.len(), height);
            self.ipfs
                .publish(&self.id, bincode::serialize(&KVMessage::Heads(heads))?)?;
        }
        Ok(())
    }

    fn apply<'a>(
        &self,
        (block, delta): &(Block, LinkedDelta),
        // tuples of (obj-cid, obj)
        adds: impl IntoIterator<Item = (Cid, Object)>,
        // tuples of (key, obj-cid)
        removes: impl IntoIterator<Item = (Vec<u8>, Cid)>,
    ) -> Result<()> {
        // TODO update tables atomically with transaction
        // tombstone removed elements
        for (key, cid) in removes.into_iter() {
            self.tombs.insert(Self::get_key_id(&key, &cid), &[])?;
        }
        for (cid, obj) in adds.into_iter() {
            // ensure dont double add or remove
            if self.tombs.contains_key(Self::get_key_id(&obj.key, &cid))? {
                continue;
            };
            // current element priority
            let prio = self
                .priorities
                .get(&obj.key)?
                .map(|v| v2u64(v))
                .transpose()?
                .unwrap_or(0);
            // current element CID at key
            let curr = self
                .elements
                .get(&obj.key)?
                .map(|b| Cid::try_from(b.as_ref()))
                .transpose()?;
            // order by priority, fall back to CID value ordering if priority equal
            if delta.delta.priority > prio
                || (delta.delta.priority == prio
                    && match curr {
                        Some(c) => c > cid,
                        _ => true,
                    })
            {
                self.elements.insert(&obj.key, cid.to_bytes())?;
                self.priorities
                    .insert(&obj.key, &u642v(delta.delta.priority))?;
            }
        }
        // find redundant heads and remove them
        // add new head
        self.heads.set(vec![(*block.cid(), delta.delta.priority)])?;
        self.heads.new_head(block.cid(), delta.prev.clone())?;
        self.ipfs.alias(block.cid().to_bytes(), Some(block.cid()))?;
        self.ipfs.insert(&block)?;

        Ok(())
    }

    #[async_recursion]
    pub(crate) async fn try_merge_heads(
        &self,
        heads: impl Iterator<Item = Cid> + Send + 'async_recursion,
    ) -> Result<()> {
        try_join_all(heads.map(|head| async move {
            // fetch head block check block is an event
            let delta_block = self.ipfs.fetch(&head, self.ipfs.peers()).await?;
            let delta: LinkedDelta = delta_block.decode()?;

            // recurse through unseen prevs first
            self.try_merge_heads(
                delta
                    .prev
                    .iter()
                    .filter_map(|p| {
                        self.heads
                            .get(p)
                            .map(|o| match o {
                                Some(_) => None,
                                None => Some(*p),
                            })
                            .transpose()
                    })
                    .collect::<Result<Vec<Cid>>>()?
                    .into_iter(),
            )
            .await?;

            let adds: Vec<(Cid, Object)> =
                try_join_all(delta.delta.add.iter().map(|c| async move {
                    let obj: Object = self.ipfs.fetch(&c, self.ipfs.peers()).await?.decode()?;
                    Ok((*c, obj)) as Result<(Cid, Object)>
                }))
                .await?;

            let removes: Vec<(Vec<u8>, Cid)> =
                try_join_all(delta.delta.rmv.iter().map(|c| async move {
                    let obj: Object = self.ipfs.fetch(&c, self.ipfs.peers()).await?.decode()?;
                    Ok((obj.key, *c)) as Result<(Vec<u8>, Cid)>
                }))
                .await?;

            self.apply(&(delta_block, delta), adds, removes)?;

            // dispatch ipfs::sync
            debug!("syncing head {}", head);
            match self.ipfs.sync(&head, self.ipfs.peers()).await {
                Ok(_) => {
                    debug!("synced head {}", head);
                    Ok(())
                }
                Err(e) => {
                    error!("failed sync head {}", e);
                    Err(anyhow!(e))
                }
            }
        }))
        .await?;
        Ok(())
    }

    pub(crate) fn request_heads(&self) -> Result<()> {
        debug!("requesting heads");
        self.ipfs
            .publish(&self.id, bincode::serialize(&KVMessage::StateReq)?)?;
        Ok(())
    }

    fn get_key_id<K: AsRef<[u8]>>(key: K, cid: &Cid) -> Vec<u8> {
        [key.as_ref(), &cid.to_bytes()].concat()
    }

    pub fn start_service(self) -> Result<Service> {
        Service::start(self)
    }
}

#[derive(Clone)]
pub struct Heads {
    heights: Tree,
    heads: Tree,
}

impl Heads {
    pub fn new(db: Db) -> Result<Self> {
        Ok(Self {
            heights: db.open_tree("heights")?,
            heads: db.open_tree("heads")?,
        })
    }

    pub fn state(&self) -> Result<(Vec<Cid>, u64)> {
        self.heads.iter().try_fold(
            (vec![], 0),
            |(mut heads, max_height), r| -> Result<(Vec<Cid>, u64)> {
                let (head, _) = r?;
                let height = v2u64(
                    self.heights
                        .get(&head)?
                        .ok_or(anyhow!("Failed to find head height"))?,
                )?;
                heads.push(head[..].try_into()?);
                Ok((heads, u64::max(max_height, height)))
            },
        )
    }

    pub fn get(&self, head: &Cid) -> Result<Option<u64>> {
        self.heights
            .get(head.to_bytes())?
            .map(|h| v2u64(h))
            .transpose()
    }

    pub fn set(&self, heights: impl IntoIterator<Item = (Cid, u64)>) -> Result<()> {
        let mut batch = Batch::default();
        for (op, height) in heights.into_iter() {
            if !self.heights.contains_key(op.to_bytes())? {
                debug!("setting head height {} {}", op, height);
                batch.insert(op.to_bytes(), &u642v(height));
            }
        }
        self.heights.apply_batch(batch)?;
        Ok(())
    }

    pub fn new_head(&self, head: &Cid, prev: impl IntoIterator<Item = Cid>) -> Result<()> {
        let mut batch = Batch::default();
        batch.insert(head.to_bytes(), &[]);
        for p in prev {
            batch.remove(p.to_bytes());
        }
        self.heads.apply_batch(batch)?;
        Ok(())
    }
}

fn v2u64<V: AsRef<[u8]>>(v: V) -> Result<u64> {
    Ok(u64::from_be_bytes(v.as_ref().try_into()?))
}

fn u642v(n: u64) -> [u8; 8] {
    n.to_be_bytes()
}
