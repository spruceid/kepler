use anyhow::Result;
use kepler_lib::libipld::{
    cbor::DagCborCodec, cid::Cid, codec::Encode, multihash::Code, raw::RawCodec,
};
use serde::{Deserialize, Serialize};

mod entries;
mod store;

use super::Block;

pub use entries::{Object, ObjectBuilder};
pub use store::{ReadResponse, Store};

#[derive(Clone)]
pub struct Service<B> {
    pub store: Store<B>,
}

impl<B> Service<B> {
    fn new(store: Store<B>) -> Self {
        Self { store }
    }

    pub async fn start(store: Store<B>) -> Result<Self> {
        Ok(Self::new(store))
    }
}

impl<B> std::ops::Deref for Service<B> {
    type Target = Store<B>;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

mod vec_cid_bin {
    use kepler_lib::libipld::cid::Cid;
    use serde::{de::Error as DeError, ser::SerializeSeq, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(vec: &[Cid], ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = ser.serialize_seq(Some(vec.len()))?;
        for cid in vec {
            seq.serialize_element(&cid.to_bytes())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D>(deser: D) -> Result<Vec<Cid>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Vec<&[u8]> = Deserialize::deserialize(deser)?;
        s.iter()
            .map(|&sc| Cid::read_bytes(sc).map_err(D::Error::custom))
            .collect()
    }
}

pub fn to_block<T: Encode<DagCborCodec>>(data: &T) -> Result<Block> {
    Block::encode(DagCborCodec, Code::Blake3_256, data)
}

pub fn to_block_raw<T: AsRef<[u8]>>(data: &T) -> Result<Block> {
    Block::encode(RawCodec, Code::Blake3_256, data.as_ref())
}

#[derive(Serialize, Deserialize, Debug)]
enum KVMessage {
    Heads(#[serde(with = "vec_cid_bin")] Vec<Cid>),
    StateReq,
}
