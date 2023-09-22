use crate::{
    hash::{hash, Hash},
    types::Metadata,
};
pub use kepler_lib::{
    authorization::{Delegation, EncodingError, HeaderEncode, Invocation, Revocation},
    libipld::cid::{
        multihash::{Code, Error as MultihashError, MultihashDigest},
        Cid,
    },
    resource::OrbitId,
};
use serde::{Deserialize, Serialize};
use serde_ipld_dagcbor::EncodeError;

#[derive(Debug)]
pub struct SerializedEvent<T>(pub T, pub(crate) Vec<u8>);

impl<T> SerializedEvent<T>
where
    T: HeaderEncode,
{
    pub fn from_header_ser(s: &str) -> Result<Self, EncodingError> {
        T::decode(s).map(|(t, s)| Self(t, s))
    }
}

pub type SDelegation = SerializedEvent<Delegation>;
pub type SInvocation = SerializedEvent<Invocation>;
pub type SRevocation = SerializedEvent<Revocation>;

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

impl Operation {
    pub fn version(self, seq: i64, epoch: Hash, epoch_seq: i64) -> VersionedOperation {
        match self {
            Self::KvWrite {
                orbit,
                key,
                value,
                metadata,
            } => VersionedOperation::KvWrite {
                orbit,
                key,
                value,
                metadata,
                seq,
                epoch,
                epoch_seq,
            },
            Self::KvDelete {
                orbit,
                key,
                version,
            } => VersionedOperation::KvDelete {
                orbit,
                key,
                version,
            },
        }
    }

    pub fn orbit(&self) -> &OrbitId {
        match self {
            Self::KvWrite { orbit, .. } => orbit,
            Self::KvDelete { orbit, .. } => orbit,
        }
    }
}

#[derive(Debug, Hash, PartialEq, Eq)]
pub(crate) enum VersionedOperation {
    KvWrite {
        orbit: OrbitId,
        key: String,
        value: Hash,
        metadata: Metadata,
        seq: i64,
        epoch: Hash,
        epoch_seq: i64,
    },
    KvDelete {
        orbit: OrbitId,
        key: String,
        version: Option<(i64, Hash, i64)>,
    },
}

#[derive(Debug)]
pub(crate) enum Event {
    Invocation(Box<SInvocation>, Vec<Operation>),
    Delegation(Box<SDelegation>),
    Revocation(Box<SRevocation>),
}

impl Event {
    pub fn hash(&self) -> Hash {
        match self {
            Event::Delegation(d) => hash(&d.1),
            Event::Invocation(i, _) => hash(&i.1),
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

#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("encoding error: {0}")]
    EncodeError(#[from] EncodeError<std::collections::TryReserveError>),
    #[error("hash error: {0}")]
    HashError(#[from] MultihashError),
}

pub(crate) fn epoch_hash(
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
                    Event::Invocation(_, ops) => hash_inv(h, orbit, ops)?,
                    Event::Delegation(_) => OneOrMany::One(h.to_cid(RAW_CODEC)),
                    Event::Revocation(_) => OneOrMany::One(h.to_cid(RAW_CODEC)),
                })
            })
            .collect::<Result<Vec<OneOrMany>, HashError>>()?,
    })?))
}

const CBOR_CODEC: u64 = 0x71;
const RAW_CODEC: u64 = 0x55;

fn hash_inv(inv_hash: &Hash, o: &OrbitId, ops: &[Operation]) -> Result<OneOrMany, HashError> {
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

    let ops = ops
        .iter()
        .filter_map(|op| match op {
            Operation::KvWrite {
                orbit,
                key,
                value,
                metadata,
            } if orbit == o => Some(Op::KvWrite {
                key,
                value: value.to_cid(CBOR_CODEC),
                metadata,
            }),
            Operation::KvDelete {
                orbit,
                key,
                version,
            } if orbit == o => Some(Op::KvDelete {
                key,
                version: version.map(|(v, h, s)| (v, h.to_cid(CBOR_CODEC), s)),
            }),
            _ => None,
        })
        .map(|op| Ok(hash(&serde_ipld_dagcbor::to_vec(&op)?).to_cid(CBOR_CODEC)))
        .collect::<Result<Vec<_>, HashError>>()?;

    Ok(if ops.is_empty() {
        OneOrMany::One(inv_hash.to_cid(RAW_CODEC))
    } else {
        let mut v = vec![inv_hash.to_cid(RAW_CODEC)];
        v.extend(ops);
        OneOrMany::Many(v)
    })
}
