use sea_orm::DbErr;

pub fn hash(data: &[u8]) -> Hash {
    Hasher::new().update(data).finalize()
}

#[derive(Debug, Clone)]
pub struct Hasher(blake3::Hasher);

impl Hasher {
    pub fn new() -> Self {
        Self(blake3::Hasher::new())
    }

    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        self.0.update(data);
        self
    }

    pub fn finalize(&self) -> Hash {
        Hash(self.0.finalize())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hash(blake3::Hash);

impl std::cmp::Ord for Hash {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_bytes().cmp(other.0.as_bytes())
    }
}

impl std::cmp::PartialOrd for Hash {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid hash len, expected 32, got {0}")]
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
        let b: [u8; 32] = value.try_into().map_err(|b: Vec<u8>| ConvertErr(b.len()))?;
        Ok(Hash(blake3::Hash::from(b)))
    }
}

impl From<Hash> for Vec<u8> {
    fn from(hash: Hash) -> Self {
        let h: [u8; 32] = hash.0.into();
        h.to_vec()
    }
}

impl AsRef<[u8; 32]> for Hash {
    fn as_ref(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }
}
