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

#[derive(Debug)]
pub(crate) enum Operation {
    KvWrite {
        key: String,
        value: Hash,
        metadata: Metadata,
    },
    KvDelete {
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
struct Epoch {
    pub seq: i64,
    pub parents: Vec<Cid>,
    pub events: Vec<Cid>,
}

#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("encoding error: {0}")]
    EncodeError(#[from] EncodeError<std::collections::TryReserveError>),
    #[error("hash error: {0}")]
    HashError(#[from] MultihashError),
}

pub fn epoch_hash(
    seq: i64,
    events: &[Event],
    parents: &[Hash],
) -> Result<(Hash, Vec<Hash>), HashError> {
    let event_hashes = events
        .iter()
        .map(|e| {
            Ok(match e {
                Event::Invocation(i) => hash_inv(&i)?.to_cid(0x71),
                Event::Delegation(d) => hash(&d.1).to_cid(0x55),
                Event::Revocation(r) => hash(&r.1).to_cid(0x55),
            })
        })
        .collect::<Result<Vec<Cid>, HashError>>()?;
    Ok((
        hash(&serde_ipld_dagcbor::to_vec(&Epoch {
            seq,
            parents: parents.iter().map(|h| h.to_cid(0x71)).collect(),
            events: event_hashes.clone(),
        })?),
        event_hashes.into_iter().map(|h| h.into()).collect(),
    ))
}

const CBOR_CODEC: u64 = 0x71;
const RAW_CODEC: u64 = 0x55;

fn hash_inv(invocation: &Invocation) -> Result<Hash, HashError> {
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

    #[derive(Debug, Serialize)]
    struct InvBlock {
        invocation: Cid,
        operations: Vec<Cid>,
    }

    Ok(hash(&serde_ipld_dagcbor::to_vec(&InvBlock {
        invocation: hash(&invocation.1).to_cid(RAW_CODEC),
        operations: match &invocation.2 {
            Some(Operation::KvWrite {
                key,
                value,
                metadata,
            }) => vec![hash(&serde_ipld_dagcbor::to_vec(&Op::KvWrite {
                key,
                value: value.to_cid(RAW_CODEC),
                metadata,
            })?)
            .to_cid(CBOR_CODEC)],
            Some(Operation::KvDelete { key, version }) => {
                vec![hash(&serde_ipld_dagcbor::to_vec(&Op::KvDelete {
                    key,
                    version: version
                        .as_ref()
                        .map(|(seq, hash)| (*seq, hash.to_cid(RAW_CODEC))),
                })?)
                .to_cid(CBOR_CODEC)]
            }
            None => vec![],
        },
    })?))
}
