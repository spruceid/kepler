use kepler_lib::libipld::cid::{
    multihash::{Blake3_256, Code, Hasher as MHasher, Multihash, MultihashDigest},
    Cid,
};
use sea_orm::DbErr;

pub fn hash(data: &[u8]) -> Hash {
    Hasher::new().update(data).finalize()
}

#[derive(Debug, Default)]
pub struct Hasher(Blake3_256);

impl Hasher {
    pub fn new() -> Self {
        Self(Blake3_256::default())
    }

    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        self.0.update(data);
        self
    }

    pub fn finalize(&mut self) -> Hash {
        Hash(Code::Blake3_256.wrap(self.0.finalize()).unwrap())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hash(Multihash);

impl Hash {
    pub fn to_cid(self, codec: u64) -> Cid {
        Cid::new_v1(codec, self.0)
    }
}

impl std::cmp::Ord for Hash {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.digest().cmp(other.0.digest())
    }
}

impl std::cmp::PartialOrd for Hash {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid hash len, expected 34, got {0}")]
pub struct ConvertErr(usize);

impl From<ConvertErr> for DbErr {
    fn from(err: ConvertErr) -> Self {
        DbErr::TryIntoErr {
            from: "Vec<u8>",
            into: "Hash",
            source: Box::new(err),
        }
    }
}

impl TryFrom<Vec<u8>> for Hash {
    type Error = ConvertErr;
    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Ok(Hash(
            Multihash::from_bytes(&value).map_err(|_| ConvertErr(value.len()))?,
        ))
    }
}

impl From<Hash> for Vec<u8> {
    fn from(hash: Hash) -> Self {
        hash.0.to_bytes()
    }
}

impl AsRef<[u8]> for Hash {
    fn as_ref(&self) -> &[u8] {
        self.0.digest()
    }
}
