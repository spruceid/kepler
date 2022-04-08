use crate::capabilities::store::AuthRef;
use crate::indexes::{AddRemoveSetStore, HeadStore};
use crate::s3::entries::{read_from_store, write_to_store};
use crate::s3::{Object, ObjectBuilder, Service};
use anyhow::Result;
use async_recursion::async_recursion;
use libipld::{cbor::DagCborCodec, cid::Cid, DagCbor};
use rocket::{futures::future::try_join_all, tokio::io::AsyncRead};
use sled::{Db, IVec, Tree};
use std::{
    collections::BTreeMap,
    convert::{TryFrom, TryInto},
};
use tracing::debug;

use super::{to_block, Block, Ipfs, KVMessage, ObjectReader};

#[derive(DagCbor)]
struct Delta {
    // max depth
    pub priority: u64,
    pub add: Vec<Cid>,
    pub rmv: Vec<(Cid, AuthRef)>,
}

impl Delta {
    pub fn new(priority: u64, add: Vec<Cid>, rmv: Vec<(Cid, AuthRef)>) -> Self {
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

struct Version<N: AsRef<[u8]>>(pub N, pub Cid);

impl<N: AsRef<[u8]>> Version<N> {
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.0.as_ref(), ".".as_bytes(), &self.1.to_bytes()].concat()
    }
}

struct Element(pub u64, pub Cid);

#[derive(thiserror::Error, Debug)]
enum ElementDeserError {
    #[error(transparent)]
    Cid(#[from] libipld::cid::Error),
    #[error("Insufficient bytes")]
    Length,
}

impl<'a> TryFrom<&'a [u8]> for Element {
    type Error = ElementDeserError;
    fn try_from(b: &[u8]) -> Result<Self, Self::Error> {
        match (
            b.get(..8)
                .map(|b| <[u8; 8]>::try_from(b).map(u64::from_be_bytes)),
            b.get(8..).map(Cid::try_from).transpose()?,
        ) {
            (Some(Ok(p)), Some(c)) => Ok(Self(p, c)),
            _ => Err(ElementDeserError::Length),
        }
    }
}

impl Element {
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.0.to_be_bytes().as_ref(), self.1.to_bytes().as_ref()].concat()
    }
}

#[derive(Clone)]
pub struct Store {
    pub id: String,
    pub ipfs: Ipfs,
    index: AddRemoveSetStore,
    priorities: Tree,
    heads: HeadStore,
}

impl Store {
    pub fn new(id: String, ipfs: Ipfs, db: &Db) -> Result<Self> {
        let index = AddRemoveSetStore::new(db, id.as_bytes())?;
        // map key to current max priority for key
        let priorities = db.open_tree("priorities")?;
        // heads tracking store
        let heads = HeadStore::new(db, "heads")?;
        Ok(Self {
            id,
            ipfs,
            index,
            priorities,
            heads,
        })
    }
    pub fn list(&self) -> impl Iterator<Item = Result<Vec<u8>>> + '_ {
        self.index.elements().filter_map(move |r| match r {
            Err(e) => Some(Err(e.into())),
            Ok((key, cid)) => match self.index.is_tombstoned(&Version(&key, cid).to_bytes()) {
                Ok(Some(false)) => Some(Ok(key)),
                Ok(Some(true) | None) => None,
                Err(e) => Some(Err(e.into())),
            },
        })
    }
    pub async fn get<N: AsRef<[u8]>>(&self, name: N) -> Result<Option<Object>> {
        let key = name;
        match self.index.element(&key)? {
            Some(cid) => Ok(
                match self.index.is_tombstoned(&Version(key, cid).to_bytes())? {
                    Some(false) => Some(self.ipfs.get_block(&cid).await?.decode()?),
                    _ => None,
                },
            ),
            None => Ok(None),
        }
    }

    pub async fn read<N>(&self, key: N) -> Result<Option<(BTreeMap<String, String>, ObjectReader)>>
    where
        N: AsRef<[u8]>,
    {
        let s3_obj = match self.get(key).await {
            Ok(Some(content)) => content,
            _ => return Ok(None),
        };
        match self
            .ipfs
            .get_block(&s3_obj.value)
            .await?
            .decode::<DagCborCodec, Vec<(Cid, u32)>>()
        {
            Ok(content) => Ok(Some((
                s3_obj.metadata,
                read_from_store(self.ipfs.clone(), content),
            ))),
            Err(_) => Ok(None),
        }
    }
    pub(crate) async fn request_heads(&self) -> Result<()> {
        debug!("requesting heads");
        self.ipfs
            .pubsub_publish(self.id.clone(), bincode::serialize(&KVMessage::StateReq)?)
            .await?;
        Ok(())
    }

    pub async fn write<N, R>(
        &self,
        add: impl IntoIterator<Item = (ObjectBuilder, R)>,
        remove: impl IntoIterator<Item = (N, Option<(u64, Cid)>, AuthRef)>,
    ) -> Result<()>
    where
        N: AsRef<[u8]>,
        R: AsyncRead + Unpin,
    {
        tracing::debug!("writing tx");
        let indexes: Vec<(Vec<u8>, Cid)> = try_join_all(add.into_iter().map(|(o, r)| async {
            // tracing::debug!("adding {:#?}", &o.key);
            let cid = write_to_store(&self.ipfs, r).await?;
            let obj = o.add_content(cid);
            let block = obj.to_block()?;
            let obj_cid = self.ipfs.put_block(block).await?;
            Ok((obj.key, obj_cid)) as Result<(Vec<u8>, Cid)>
        }))
        .await?
        .into_iter()
        .collect();
        self.index(indexes, remove).await
    }

    pub async fn index<N, M>(
        &self,
        // tuples of (obj-name, content cid, auth id)
        add: impl IntoIterator<Item = (N, Cid)>,
        // tuples of (key, opt (priority, obj-cid), auth id))
        remove: impl IntoIterator<Item = (M, Option<(u64, Cid)>, AuthRef)>,
    ) -> Result<()>
    where
        N: AsRef<[u8]>,
        M: AsRef<[u8]>,
    {
        let (heads, height) = self.heads.get_heads()?;
        let height = if heads.is_empty() && height == 0 {
            0
        } else {
            height + 1
        };
        let adds: (Vec<(N, Cid)>, Vec<Cid>) =
            add.into_iter().map(|(key, cid)| ((key, cid), cid)).unzip();
        type Rmvs<K> = (Vec<(K, Cid)>, Vec<(Cid, AuthRef)>);
        let rmvs: Rmvs<M> = remove
            .into_iter()
            .map(|(key, version, auth)| {
                Ok(match version {
                    Some((_, cid)) => ((key, cid), (cid, auth)),
                    None => {
                        let cid = self
                            .index
                            .element(&key)?
                            .ok_or_else(|| anyhow!("Failed to find Object ID for key"))?;
                        ((key, cid), (cid, auth))
                    }
                })
            })
            .collect::<Result<Vec<((M, Cid), (Cid, AuthRef))>>>()?
            .into_iter()
            .unzip();
        let delta = LinkedDelta::new(heads, Delta::new(height, adds.1, rmvs.1));
        let block = delta.to_block()?;
        // apply/pin root/update heads
        self.apply(&(block, delta), adds.0, rmvs.0).await?;

        // broadcast
        self.broadcast_heads().await?;
        Ok(())
    }

    pub(crate) async fn broadcast_heads(&self) -> Result<()> {
        let (heads, height) = self.heads.get_heads()?;
        if !heads.is_empty() {
            debug!(
                "broadcasting {} heads at maxheight {} on {}",
                heads.len(),
                height,
                self.id,
            );
            self.ipfs
                .pubsub_publish(
                    self.id.clone(),
                    bincode::serialize(&KVMessage::Heads(heads))?,
                )
                .await?;
        }
        Ok(())
    }

    async fn apply<N, M>(
        &self,
        (block, delta): &(Block, LinkedDelta),
        // tuples of (obj-cid, obj)
        adds: impl IntoIterator<Item = (N, Cid)>,
        // tuples of (key, obj-cid)
        removes: impl IntoIterator<Item = (M, Cid)>,
    ) -> Result<()>
    where
        N: AsRef<[u8]>,
        M: AsRef<[u8]>,
    {
        // TODO update tables atomically with transaction
        // tombstone removed elements
        for (key, cid) in removes.into_iter() {
            self.index.set_tombstone(&Version(&key, cid).to_bytes())?;
        }
        for (key, cid) in adds.into_iter() {
            // ensure dont double add or remove
            if self
                .index
                .is_tombstoned(&Version(&key, cid).to_bytes())?
                .unwrap_or(false)
            {
                continue;
            };

            // current element priority and current element CID at key
            let (prio, curr) = match self.index.element(&key)? {
                Some(Element(p, c)) => (p, Some(c)),
                None => (0, None),
            };
            // order by priority, fall back to CID value ordering if priority equal
            if delta.delta.priority > prio
                || (delta.delta.priority == prio
                    && match curr {
                        Some(c) => c > cid,
                        _ => true,
                    })
            {
                self.index
                    .set_element(&key, &Element(delta.delta.priority, cid).to_bytes())?;
            }
        }
        // find redundant heads and remove them
        // add new head
        self.heads
            .set_heights([(*block.cid(), delta.delta.priority)])?;
        self.heads.new_heads([*block.cid()], delta.prev.clone())?;
        self.ipfs.put_block(block.clone()).await?;

        Ok(())
    }

    #[async_recursion]
    pub(crate) async fn try_merge_heads(
        &self,
        heads: impl Iterator<Item = Cid> + Send + 'async_recursion,
    ) -> Result<()> {
        try_join_all(heads.map(|head| async move {
            // fetch head block check block is an event
            let delta_block = self.ipfs.get_block(&head).await?;
            let delta: LinkedDelta = delta_block.decode()?;

            // recurse through unseen prevs first
            self.try_merge_heads(
                delta
                    .prev
                    .iter()
                    .filter_map(|p| {
                        self.heads
                            .get_height(p)
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

            let adds: Vec<(Vec<u8>, Cid)> =
                try_join_all(delta.delta.add.iter().map(|c| async move {
                    let obj: Object = self.ipfs.get_block(c).await?.decode()?;
                    Ok((obj.key, *c)) as Result<(Vec<u8>, Cid)>
                }))
                .await?;

            let removes: Vec<(Vec<u8>, Cid)> =
                try_join_all(delta.delta.rmv.iter().map(|c| async move {
                    let obj: Object = self.ipfs.get_block(&c.0).await?.decode()?;
                    Ok((obj.key, c.0)) as Result<(Vec<u8>, Cid)>
                }))
                .await?;

            // TODO verify authz stuff

            self.apply(&(delta_block, delta), adds, removes).await?;

            // dispatch ipfs::sync
            debug!("syncing head {}", head);

            self.ipfs.insert_pin(&head, true).await?;
            Ok(()) as Result<()>
        }))
        .await?;
        Ok(())
    }
    pub async fn start_service(self) -> Result<Service> {
        Service::start(self).await
    }
}
