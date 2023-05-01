use crate::hashes::Hasher;
pub mod actor;
pub mod delegation;
pub mod epoch;
pub mod invocation;
pub mod revocation;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpochId {
    seq: u64,
    hash: [u8; 32],
}

impl EpochId {
    pub fn new(seq: u64, hash: [u8; 32]) -> Self {
        Self { seq, hash }
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn hash(&self) -> &[u8; 32] {
        &self.hash
    }

    pub fn to_bytes(&self) -> [u8; 40] {
        let mut bytes = [0u8; 40];
        bytes[..8].copy_from_slice(&self.seq.to_be_bytes());
        bytes[8..].copy_from_slice(&self.hash);
        bytes
    }

    pub fn from_bytes(bytes: &[u8; 40]) -> Self {
        Self::new(
            u64::from_be_bytes(bytes[..8].try_into().unwrap()),
            bytes[8..].try_into().unwrap(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionId {
    epoch_id: EpochId,
    inner_seq: u64,
}

impl TransactionId {
    pub fn new(epoch_id: EpochId, inner_seq: u64) -> Self {
        Self {
            epoch_id,
            inner_seq,
        }
    }

    pub fn epoch_id(&self) -> &EpochId {
        &self.epoch_id
    }

    pub fn seq(&self) -> u64 {
        self.inner_seq
    }

    pub fn to_bytes(&self) -> [u8; 48] {
        let mut bytes = [0u8; 48];
        bytes[..8].copy_from_slice(&self.epoch_id.seq().to_be_bytes());
        bytes[8..40].copy_from_slice(self.epoch_id.hash());
        bytes[40..].copy_from_slice(&self.inner_seq.to_be_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8; 48]) -> Self {
        Self::new(
            EpochId::from_bytes(bytes[..40].try_into().unwrap()),
            u64::from_be_bytes(bytes[40..].try_into().unwrap()),
        )
    }
}
