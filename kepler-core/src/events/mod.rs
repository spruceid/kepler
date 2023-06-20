use crate::{
    hash::{hash, Hash},
    types::Metadata,
    util::{DelegationInfo, InvocationInfo, RevocationInfo},
};
pub use kepler_lib::{
    authorization::{HeaderEncode, KeplerDelegation, KeplerInvocation, KeplerRevocation},
    libipld::cid::{
        multihash::{Code, Error as MultihashError, MultihashDigest},
        Cid,
    },
    resource::OrbitId,
};
use serde::{Deserialize, Serialize};
use serde_ipld_dagcbor::EncodeError;

#[derive(Debug)]
pub struct Delegation(pub DelegationInfo, pub(crate) Vec<u8>);

#[derive(Debug)]
pub struct Revocation(pub RevocationInfo, pub(crate) Vec<u8>);

#[derive(Debug)]
pub struct Invocation(
    pub InvocationInfo,
    pub(crate) Vec<u8>,
    pub(crate) Option<Operation>,
);

#[derive(Debug, Hash, PartialEq, Eq)]
pub(crate) enum Operation {
    KvWrite {
        orbit: OrbitId,
        key: String,
        value: Hash,
        metadata: Metadata,
    },
    KvDelete {
        orbit: OrbitId,
        key: String,
        version: Option<(i64, Hash, i64)>,
    },
}

#[derive(Debug)]
pub enum Event {
    Invocation(Box<Invocation>),
    Delegation(Box<Delegation>),
    Revocation(Box<Revocation>),
}

impl Event {
    pub fn hash(&self) -> Hash {
        match self {
            Event::Delegation(d) => hash(&d.1),
            Event::Invocation(i) => hash(&i.1),
            Event::Revocation(r) => hash(&r.1),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(Cid),
    Many(Vec<Cid>),
}

#[derive(Debug, Serialize, Deserialize)]
struct Epoch {
    pub parents: Vec<Cid>,
    pub events: Vec<OneOrMany>,
}

#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("encoding error: {0}")]
    EncodeError(#[from] EncodeError<std::collections::TryReserveError>),
    #[error("hash error: {0}")]
    HashError(#[from] MultihashError),
}

pub fn epoch_hash(
    orbit: &OrbitId,
    events: &[&(Hash, Event)],
    parents: &[Hash],
) -> Result<Hash, HashError> {
    Ok(hash(&serde_ipld_dagcbor::to_vec(&Epoch {
        parents: parents.iter().map(|h| h.to_cid(0x71)).collect(),
        events: events
            .iter()
            .map(|(h, e)| {
                Ok(match e {
                    Event::Invocation(i) => hash_inv(&h, &i, orbit)?,
                    Event::Delegation(_) => OneOrMany::One(h.to_cid(RAW_CODEC)),
                    Event::Revocation(_) => OneOrMany::One(h.to_cid(RAW_CODEC)),
                })
            })
            .collect::<Result<Vec<OneOrMany>, HashError>>()?,
    })?))
}

const CBOR_CODEC: u64 = 0x71;
const RAW_CODEC: u64 = 0x55;

fn hash_inv(inv_hash: &Hash, inv: &Invocation, o: &OrbitId) -> Result<OneOrMany, HashError> {
    #[derive(Debug, Serialize)]
    #[serde(untagged)]
    enum Op<'a> {
        KvWrite {
            key: &'a str,
            value: Cid,
            metadata: &'a Metadata,
        },
        KvDelete {
            key: &'a str,
            version: Option<(i64, Cid, i64)>,
        },
    }

    Ok(match &inv.2 {
        Some(Operation::KvWrite {
            orbit,
            key,
            value,
            metadata,
        }) if orbit == o => OneOrMany::Many(vec![
            inv_hash.to_cid(RAW_CODEC),
            hash(&serde_ipld_dagcbor::to_vec(&Op::KvWrite {
                key,
                value: value.to_cid(CBOR_CODEC),
                metadata,
            })?)
            .to_cid(CBOR_CODEC),
        ]),
        Some(Operation::KvDelete {
            orbit,
            key,
            version,
        }) if orbit == o => OneOrMany::Many(vec![
            inv_hash.to_cid(RAW_CODEC),
            hash(&serde_ipld_dagcbor::to_vec(&Op::KvDelete {
                key,
                version: version
                    .as_ref()
                    .map(|(seq, hash, es)| (*seq, hash.to_cid(CBOR_CODEC), *es)),
            })?)
            .to_cid(CBOR_CODEC),
        ]),
        _ => OneOrMany::One(inv_hash.to_cid(RAW_CODEC)),
    })
}
