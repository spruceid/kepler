use super::codec::SupportedCodecs;
use anyhow::Result;
use cid::{Cid, Version};
use multihash::{Code, MultihashDigest};
use rocksdb::{Options, DB};
use std::{
    convert::TryFrom,
    io::Read,
    path::Path,
    sync::{Arc, Mutex},
};

pub trait ContentAddressedStorage {
    type Error;
    fn put<C: Read>(&self, content: C, codec: SupportedCodecs) -> Result<Cid, Self::Error>;
    fn get(&self, address: Cid) -> Result<Option<Vec<u8>>, Self::Error>;
    fn delete(&self, address: Cid) -> Result<(), Self::Error>;
}

pub struct CASDB(Arc<Mutex<DB>>);

impl CASDB {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(Self(Arc::new(Mutex::new(DB::open_default(path)?))))
    }
}

impl ContentAddressedStorage for CASDB {
    type Error = anyhow::Error;
    fn put<C: Read>(&self, content: C, codec: SupportedCodecs) -> Result<Cid, Self::Error> {
        let c: Vec<u8> = content.bytes().filter_map(|b| b.ok()).collect();
        let cid = Cid::new(Version::V1, codec as u64, Code::Blake3_256.digest(&c))?;

        self.0
            .lock()
            .map_err(|e| anyhow!(format!("{}", e)))?
            .put(&cid.to_bytes(), &c)?;

        Ok(cid)
    }
    fn get(&self, address: Cid) -> Result<Option<Vec<u8>>, Self::Error> {
        match self
            .0
            .lock()
            .map_err(|e| anyhow!(format!("{}", e)))?
            .get(address.to_bytes())
        {
            Ok(Some(content)) => {
                if Code::try_from(address.hash().code())?.digest(&content) != *address.hash() {
                    Err(anyhow!("Invalid Content Address"))
                } else {
                    Ok(Some(content.to_vec()))
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn delete(&self, address: Cid) -> Result<(), Self::Error> {
        self.0
            .lock()
            .map_err(|e| anyhow!(format!("{}", e)))?
            .delete(address.to_bytes())
            .map_err(|e| e.into())
    }
}
