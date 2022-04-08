use anyhow::Result;
use libipld::{cid::Cid, multibase::Base};
use sled::{Batch, Db, Tree};

use crate::config;

#[derive(Clone)]
pub enum KV {
    Sled(Tree),
    // DynamoDB(Box<KVDynamoDB>)
}

impl KV {
    pub async fn new(
        orbit_id: Cid,
        subsystem_name: String,
        table_name: String,
        config: config::IndexStorage,
    ) -> Result<Self> {
        match config {
            config::IndexStorage::Local(c) => {
                let path = c
                    .path
                    .join(orbit_id.to_string_of_base(Base::Base58Btc)?)
                    .join(subsystem_name)
                    .join(table_name)
                    .join("db.sled");
                tokio::fs::create_dir_all(&path).await?;
                let db = sled::open(path)?;
                let elements = db.open_tree("elements".as_bytes())?;
                Ok(KV::Sled(elements))
            }
            config::IndexStorage::DynamoDB(c) => panic!(), //KV::DynamoDB(),
        }
    }

    pub async fn get<N: AsRef<[u8]>>(&self, key: N) -> Result<Option<Vec<u8>>> {
        match self {
            KV::Sled(c) => Ok(c.get(key)?.map(|v| v.to_vec())),
        }
    }

    pub async fn insert<N: AsRef<[u8]>, E: AsRef<[u8]> + ?Sized>(
        &self,
        key: N,
        element: &E,
    ) -> Result<Option<Vec<u8>>> {
        match self {
            KV::Sled(c) => Ok(c
                .insert(key, element.as_ref())
                .map(|e| e.map(|v| v.to_vec()))?),
        }
    }

    pub async fn insert_batch(&self, batch: Vec<(Vec<u8>, Vec<u8>)>) -> Result<()> {
        match self {
            KV::Sled(c) => {
                let mut sled_batch = Batch::default();
                for (op, height) in batch.into_iter() {
                    sled_batch.insert(op, height);
                }
                c.apply_batch(sled_batch)?;
            }
        }
        Ok(())
    }
    pub async fn remove_batch(&self, batch: Vec<Vec<u8>>) -> Result<()> {
        match self {
            KV::Sled(c) => {
                let mut sled_batch = Batch::default();
                for op in batch.into_iter() {
                    sled_batch.remove(op);
                }
                c.apply_batch(sled_batch)?;
            }
        }
        Ok(())
    }

    pub async fn contains_key<N: AsRef<[u8]>>(&self, key: N) -> Result<bool> {
        match self {
            KV::Sled(c) => Ok(c.contains_key(key)?),
        }
    }

    pub async fn elements(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        match self {
            KV::Sled(c) => Ok(c
                .iter()
                .map(|e| e.map(|ee| (ee.0.to_vec(), ee.1.to_vec())))
                .collect::<Result<Vec<(Vec<u8>, Vec<u8>)>, sled::Error>>()?),
        }
    }
}
