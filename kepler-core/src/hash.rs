use kepler_lib::libipld::cid::{
    multihash::{Blake3_256, Code, Hasher as MHasher, Multihash, MultihashDigest},
    Cid,
};
use sea_orm::entity::prelude::*;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

impl From<Multihash> for Hash {
    fn from(value: Multihash) -> Self {
        Self(value)
    }
}

impl From<Cid> for Hash {
    fn from(value: Cid) -> Self {
        Self(*value.hash())
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

impl From<Hash> for Value {
    fn from(hash: Hash) -> Self {
        Value::Bytes(Some(Box::new(hash.into())))
    }
}

impl sea_orm::TryGetable for Hash {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &QueryResult,
        idx: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        let vec: Vec<u8> = res.try_get_by(idx).map_err(sea_orm::TryGetError::DbErr)?;
        Hash::try_from(vec).map_err(|e| {
            sea_orm::TryGetError::DbErr(DbErr::TryIntoErr {
                from: "Vec<u8>",
                into: "Hash",
                source: Box::new(e),
            })
        })
    }
}

impl sea_orm::sea_query::ValueType for Hash {
    fn try_from(v: Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            Value::Bytes(Some(x)) => Ok(<Hash as TryFrom<Vec<u8>>>::try_from(*x)
                .map_err(|_| sea_orm::sea_query::ValueTypeErr)?),
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        stringify!(Hash).to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::Bytes
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Binary(sea_orm::sea_query::table::BlobSize::Blob(None))
    }
}

impl sea_orm::sea_query::Nullable for Hash {
    fn null() -> Value {
        Value::Bytes(None)
    }
}

impl sea_orm::TryFromU64 for Hash {
    fn try_from_u64(_: u64) -> Result<Self, DbErr> {
        Err(DbErr::ConvertFromU64(stringify!($type)))
    }
}
