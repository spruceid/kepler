use crate::indexes::{AddRemoveSetStore, HeadStore};
use crate::kv::{Object, ObjectBuilder, Service};
use crate::storage::{Content, ImmutableStore};
use anyhow::Result;
use futures::{
    io::AsyncRead,
    stream::{self, StreamExt, TryStreamExt},
};
use kepler_lib::libipld::{cid::Cid, multibase::Base, multihash::Code, DagCbor};
use rocket::futures::future::try_join_all;
use std::{collections::BTreeMap, convert::TryFrom};
use tracing::instrument;

use super::{to_block, Block};
use crate::config;

#[derive(DagCbor)]
struct Delta {
    // max depth
    pub priority: u64,
    pub add: Vec<Cid>,
    pub rmv: Vec<(Cid, Cid)>,
}

impl Delta {
    pub fn new(priority: u64, add: Vec<Cid>, rmv: Vec<(Cid, Cid)>) -> Self {
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
    Cid(#[from] kepler_lib::libipld::cid::Error),
    #[error("Insufficient bytes")]
    Length,
}

impl TryFrom<Vec<u8>> for Element {
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
pub struct Store<B> {
    pub id: String,
    blocks: B,
    index: AddRemoveSetStore,
    heads: HeadStore,
}

impl<B> Store<B> {
    pub async fn new(orbit_id: Cid, blocks: B, config: config::IndexStorage) -> Result<Self> {
        let index = AddRemoveSetStore::new(orbit_id, "kv".to_string(), config.clone()).await?;
        // heads tracking store
        let heads = HeadStore::new(orbit_id, "kv".to_string(), "heads".to_string(), config).await?;
        Ok(Self {
            id: orbit_id.to_string_of_base(Base::Base58Btc)?,
            blocks,
            index,
            heads,
        })
    }

    pub fn blocks(&self) -> &B {
        &self.blocks
    }

    #[instrument(name = "kv::list", skip_all)]
    pub async fn list(&self) -> impl Iterator<Item = Result<Vec<u8>>> {
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
}

pub struct ReadResponse<R>(pub BTreeMap<String, String>, pub R);

impl<B> Store<B>
where
    B: ImmutableStore + 'static,
{
    #[instrument(name = "kv::get", skip_all)]
    pub async fn get<N>(&self, name: N) -> Result<Option<Object>>
    where
        N: AsRef<[u8]>,
    {
        match self.index.element(&name).await? {
            Some(Element(_, cid)) => match self
                .index
                .is_tombstoned(&Version(name, cid).to_bytes())
                .await?
            {
                false => self
                    .blocks
                    .read_to_vec(cid.hash())
                    .await?
                    .map(|v| Block::new(cid, v)?.decode())
                    .transpose(),
                _ => Ok(None),
            },
            None => Ok(None),
        }
    }

    #[instrument(name = "kv::read", skip_all)]
    pub async fn read<N>(&self, key: N) -> Result<Option<ReadResponse<Content<B::Readable>>>>
    where
        N: AsRef<[u8]>,
    {
        let kv_obj = match self.get(key).await {
            Ok(Some(content)) => content,
            _ => return Ok(None),
        };
        match self.blocks.read(kv_obj.value.hash()).await? {
            Some(r) => Ok(Some(ReadResponse(kv_obj.metadata, r))),
            None => Err(anyhow!("Indexed contents missing from block store")),
        }
    }

    #[instrument(name = "kv::write", skip_all)]
    pub async fn write<N, R>(
        &self,
        add: impl IntoIterator<Item = (ObjectBuilder, R)>,
        remove: impl IntoIterator<Item = (N, Option<(u64, Cid)>, Cid)>,
        // TODO return list of new heads to be broadcast?
    ) -> Result<()>
    where
        N: AsRef<[u8]>,
        R: AsyncRead + Send,
    {
        tracing::debug!("writing tx");
        let indexes: Vec<(Vec<u8>, Cid)> = try_join_all(add.into_iter().map(|(o, r)| async {
            // tracing::debug!("adding {:#?}", &o.key);
            // store aaalllllll the content bytes under 1 CID
            let cid = Cid::new_v1(0x55, self.blocks.write(r, Code::Blake3_256).await?);
            let obj = o.add_content(cid);
            let block = obj.to_block()?;
            let obj_cid = Cid::new_v1(
                block.cid().codec(),
                self.blocks
                    .write(block.data(), block.cid().hash().code().try_into()?)
                    .await?,
            );
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
        remove: impl IntoIterator<Item = (M, Option<(u64, Cid)>, Cid)>,
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
        type Rmvs<K> = (Vec<(K, Cid)>, Vec<(Cid, Cid)>);
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
            .try_collect::<Vec<((M, Cid), (Cid, Cid))>>()
            .await?
            .into_iter()
            .unzip();
        let delta = LinkedDelta::new(heads, Delta::new(height, adds.1, rmvs.1));
        let block = delta.to_block()?;
        // apply/pin root/update heads
        self.apply(&(block, delta), adds.0, rmvs.0).await?;

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
        self.blocks
            .write(block.data(), block.cid().hash().code().try_into()?)
            .await?;

        Ok(())
    }
    pub async fn start_service(self) -> Result<Service<B>> {
        Service::start(self).await
    }
}
