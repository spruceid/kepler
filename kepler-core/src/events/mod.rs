use crate::{
    hash::{hash, Hash},
    models::kv_write::Metadata,
};
pub use kepler_lib::{
    authorization::{KeplerDelegation, KeplerInvocation, KeplerRevocation},
    libipld::cid::{
        multihash::{Code, Error as MultihashError, MultihashDigest},
        Cid,
    },
};
use serde::{Deserialize, Serialize};
use serde_ipld_dagcbor::EncodeError;

#[derive(Debug)]
pub struct Delegation(pub KeplerDelegation, pub Vec<u8>);

#[derive(Debug)]
pub struct Invocation(pub KeplerInvocation, pub Vec<u8>, pub Option<Operation>);

#[derive(Debug)]
pub enum Operation {
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
pub struct Revocation(pub KeplerRevocation, pub Vec<u8>);

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
            hash(match e {
                Event::Invocation(i) => &i.1,
                Event::Delegation(d) => &d.1,
                Event::Revocation(r) => &r.1,
            })
        })
        .collect::<Vec<_>>();
    Ok((
        hash(&serde_ipld_dagcbor::to_vec(&Epoch {
            seq,
            parents: parents.iter().map(|h| h.to_cid(0x55)).collect(),
            events: event_hashes.iter().map(|h| h.to_cid(0x55)).collect(),
        })?),
        event_hashes,
    ))
}
