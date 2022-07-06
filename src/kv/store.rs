use crate::capabilities::store::AuthRef;
use crate::indexes::{AddRemoveSetStore, HeadStore};
use crate::kv::entries::{read_from_store, write_to_store};
use crate::kv::{Object, ObjectBuilder, Service};
use anyhow::Result;
use async_recursion::async_recursion;
use futures::stream::{self, StreamExt, TryStreamExt};
use kepler_lib::libipld::{cbor::DagCborCodec, cid::Cid, multibase::Base, DagCbor};
use rocket::{futures::future::try_join_all, tokio::io::AsyncRead};
use std::{collections::BTreeMap, convert::TryFrom};
use tracing::{debug, instrument};

use super::{to_block, Block, Ipfs, KVMessage, ObjectReader};
use crate::config;

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

impl<'a> TryFrom<Vec<u8>> for Element {
    type Error = ElementDeserError;
    fn try_from(b: Vec<u8>) -> Result<Self, Self::Error> {
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
    heads: HeadStore,
}

impl Store {
    pub async fn new(orbit_id: Cid, ipfs: Ipfs, config: config::IndexStorage) -> Result<Self> {
        let index = AddRemoveSetStore::new(orbit_id, "kv".to_string(), config.clone()).await?;
        // heads tracking store
        let heads = HeadStore::new(orbit_id, "kv".to_string(), "heads".to_string(), config).await?;
        Ok(Self {
            id: orbit_id.to_string_of_base(Base::Base58Btc)?,
            ipfs,
            index,
            heads,
        })
    }

    #[instrument(name = "kv::list", skip_all)]
    pub async fn list(&self) -> impl Iterator<Item = Result<Vec<u8>>> + '_ {
        let elements = match self.index.elements().await {
            Ok(e) => e,
            Err(e) => return vec![Err(e)].into_iter(),
        };
        stream::iter(elements)
            .filter_map(move |r| async move {
                match r {
                    Err(e) => Some(Err(e.into())),
                    Ok((key, Element(_, cid))) => {
                        match self
                            .index
                            .is_tombstoned(&Version(&key, cid).to_bytes())
                            .await
                        {
                            Ok(false) => Some(Ok(key)),
                            Ok(true) => None,
                            Err(e) => Some(Err(e)),
                        }
                    }
                }
            })
            .collect::<Vec<Result<Vec<u8>>>>()
            .await
            .into_iter()
    }

    #[instrument(name = "kv::get", skip_all)]
    pub async fn get<N: AsRef<[u8]>>(&self, name: N) -> Result<Option<Object>> {
        let key = name;
        match self.index.element(&key).await? {
            Some(Element(_, cid)) => Ok(
                match self
                    .index
                    .is_tombstoned(&Version(key, cid).to_bytes())
                    .await?
                {
                    false => Some(self.ipfs.get_block(&cid).await?.decode()?),
                    _ => None,
                },
            ),
            None => Ok(None),
        }
    }

    #[instrument(name = "kv::read", skip_all)]
    pub async fn read<N>(&self, key: N) -> Result<Option<(BTreeMap<String, String>, ObjectReader)>>
    where
        N: AsRef<[u8]>,
    {
        let kv_obj = match self.get(key).await {
            Ok(Some(content)) => content,
            _ => return Ok(None),
        };
        match self
            .ipfs
            .get_block(&kv_obj.value)
            .await?
            .decode::<DagCborCodec, Vec<(Cid, u32)>>()
        {
            Ok(content) => Ok(Some((
                kv_obj.metadata,
                read_from_store(self.ipfs.clone(), content),
            ))),
            Err(_) => Ok(None),
        }
    }

    #[instrument(name = "kv::request_heads", skip_all)]
    pub(crate) async fn request_heads(&self) -> Result<()> {
        debug!("requesting heads");
        self.ipfs
            .pubsub_publish(self.id.clone(), bincode::serialize(&KVMessage::StateReq)?)
            .await?;
        Ok(())
    }

    #[instrument(name = "kv::write", skip_all)]
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

    #[instrument(name = "kv::index", skip_all)]
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
        let (heads, height) = self.heads.get_heads().await?;
        let height = if heads.is_empty() && height == 0 {
            0
        } else {
            height + 1
        };
        let adds: (Vec<(N, Cid)>, Vec<Cid>) =
            add.into_iter().map(|(key, cid)| ((key, cid), cid)).unzip();
        type Rmvs<K> = (Vec<(K, Cid)>, Vec<(Cid, AuthRef)>);
        let rmvs: Rmvs<M> = stream::iter(remove.into_iter().map(Ok).collect::<Vec<Result<_>>>())
            .and_then(|(key, version, auth)| async move {
                Ok(match version {
                    Some((_, cid)) => ((key, cid), (cid, auth)),
                    None => {
                        let Element(_, cid) = self
                            .index
                            .element(&key)
                            .await?
                            .ok_or_else(|| anyhow!("Failed to find Object ID for key"))?;
                        ((key, cid), (cid, auth))
                    }
                })
            })
            .try_collect::<Vec<((M, Cid), (Cid, AuthRef))>>()
            .await?
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

    #[instrument(name = "kv::broadcast_heads", skip_all)]
    pub(crate) async fn broadcast_heads(&self) -> Result<()> {
        let (heads, height) = self.heads.get_heads().await?;
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
            self.index
                .set_tombstone(&Version(&key, cid).to_bytes())
                .await?;
        }
        for (key, cid) in adds.into_iter() {
            // ensure dont double add or remove
            if self
                .index
                .is_tombstoned(&Version(&key, cid).to_bytes())
                .await?
            {
                continue;
            };

            // current element priority and current element CID at key
            let (prio, curr) = match self.index.element(&key).await? {
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
                    .set_element(&key, &Element(delta.delta.priority, cid).to_bytes())
                    .await?;
            }
        }
        // find redundant heads and remove them
        // add new head
        self.heads
            .set_heights([(*block.cid(), delta.delta.priority)])
            .await?;
        self.heads
            .new_heads([*block.cid()], delta.prev.clone())
            .await?;
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
                stream::iter(delta.prev.iter().map(Ok).collect::<Vec<Result<_>>>())
                    .try_filter_map(|p| async move {
                        self.heads.get_height(p).await.map(|o| match o {
                            Some(_) => None,
                            None => Some(p),
                        })
                    })
                    .try_collect::<Vec<Cid>>()
                    .await?
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
