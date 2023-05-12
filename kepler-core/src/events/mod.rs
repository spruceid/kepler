use crate::hash::{hash, Hash};
pub use kepler_lib::{
    authorization::{KeplerDelegation, KeplerInvocation, KeplerRevocation},
    libipld::cid::{
        multihash::{Code, Error as MultihashError, MultihashDigest},
        Cid,
    },
};
use serde::{Deserialize, Serialize};
use serde_ipld_dagcbor::{to_vec, EncodeError};

#[derive(Debug)]
pub struct Delegation(pub KeplerDelegation, pub Vec<u8>);

#[derive(Debug)]
pub struct Invocation(pub KeplerInvocation, pub Vec<u8>, pub Option<Operation>);

#[derive(Debug)]
pub enum Operation {
    KvWrite { key: String, value: Vec<u8> },
    KvDelete { key: String },
}

#[derive(Debug)]
pub struct Revocation(pub KeplerRevocation, pub Vec<u8>);

#[derive(Debug)]
pub enum Event {
    Invocation(Invocation),
    Delegation(Delegation),
    Revocation(Revocation),
}

#[derive(Debug, Serialize, Deserialize)]
struct Epoch {
    pub seq: u64,
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
    seq: u64,
    events: &[Event],
    parents: &[Hash],
) -> Result<(Hash, Vec<Hash>), HashError> {
    let event_hashes = events
        .iter()
        .map(|e| {
            hash(match e {
                Event::Invocation(Invocation(_, s, _)) => s,
                Event::Delegation(Delegation(_, s)) => s,
                Event::Revocation(Revocation(_, s)) => s,
            })
        })
        .collect::<Vec<_>>();
    Ok((
        hash(&serde_ipld_dagcbor::to_vec(&Epoch {
            seq,
            parents: parents
                .iter()
                .map(|h| Ok(Cid::new_v1(0x55, Code::Blake3_256.wrap(h.as_ref())?)))
                .collect::<Result<Vec<Cid>, MultihashError>>()?,
            events: event_hashes
                .iter()
                .map(|h| Ok(Cid::new_v1(0x55, Code::Blake3_256.wrap(h.as_ref())?)))
                .collect::<Result<Vec<Cid>, MultihashError>>()?,
        })?),
        event_hashes,
    ))
}
