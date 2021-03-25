use cid::{Cid, Codec};
use std::io::Read;

pub trait ContentAddressedStorage {
    type Error;
    fn put<C: Read>(&self, content: C, codec: Codec) -> Result<Cid, Self::Error>;
    fn get(&self, address: Cid) -> Result<Option<Vec<u8>>, Self::Error>;
    fn delete(&self, address: Cid) -> Result<(), Self::Error>;
}
