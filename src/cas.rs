use multihash::Multihash;
use std::io::Read;

pub trait ContentAddressedStorage {
    type Error;
    fn put<C: Read>(&self, content: C) -> Result<Multihash, Self::Error>;
    fn get(&self, digest: Multihash) -> Result<Option<Vec<u8>>, Self::Error>;
    fn delete(&self, digest: Multihash) -> Result<(), Self::Error>;
}
