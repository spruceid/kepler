use crate::{
    hash::{hash, Hash},
    models::kv_write::Metadata,
    util::{
        DelegationError, DelegationInfo, InvocationError, InvocationInfo, RevocationError,
        RevocationInfo,
    },
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
        version: Option<(i64, Hash)>,
    },
}

#[derive(Debug)]
pub enum Event {
    Invocation(Box<Invocation>),
    Delegation(Box<Delegation>),
    Revocation(Box<Revocation>),
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

pub fn epoch_hash(orbit: &OrbitId, events: &[&Event], parents: &[Hash]) -> Result<Hash, HashError> {
    Ok(hash(&serde_ipld_dagcbor::to_vec(&Epoch {
        parents: parents.iter().map(|h| h.to_cid(0x71)).collect(),
        events: events
            .iter()
            .map(|e| {
                Ok(match e {
                    Event::Invocation(i) => hash_inv(&i, orbit)?,
                    Event::Delegation(d) => OneOrMany::One(hash(&d.1).to_cid(RAW_CODEC)),
                    Event::Revocation(r) => OneOrMany::One(hash(&r.1).to_cid(RAW_CODEC)),
                })
            })
            .collect::<Result<Vec<OneOrMany>, HashError>>()?,
    })?))
}

const CBOR_CODEC: u64 = 0x71;
const RAW_CODEC: u64 = 0x55;

fn hash_inv(invocation: &Invocation, o: &OrbitId) -> Result<OneOrMany, HashError> {
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
            version: Option<(i64, Cid)>,
        },
    }

    let inv_hash = hash(&invocation.1).to_cid(RAW_CODEC);

    Ok(match &invocation.2 {
        Some(Operation::KvWrite {
            orbit,
            key,
            value,
            metadata,
        }) if orbit == o => OneOrMany::Many(vec![
            inv_hash,
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
            inv_hash,
            hash(&serde_ipld_dagcbor::to_vec(&Op::KvDelete {
                key,
                version: version
                    .as_ref()
                    .map(|(seq, hash)| (*seq, hash.to_cid(CBOR_CODEC))),
            })?)
            .to_cid(CBOR_CODEC),
        ]),
        _ => OneOrMany::One(inv_hash),
    })
}
