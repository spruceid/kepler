use super::codec::SupportedCodecs;
use anyhow::Result;
use cid::{Cid, Version};
use ipfs_embed::{DefaultParams, Ipfs};
use multihash::{Code, MultihashDigest};
use rocket::tokio::io::AsyncRead;
use std::{
    convert::TryFrom,
    io::Read,
    path::Path,
    sync::{Arc, Mutex},
};

#[rocket::async_trait]
pub trait ContentAddressedStorage: Send + Sync {
    type Error;
    async fn put<C: AsyncRead + Send>(
        &self,
        content: C,
        codec: SupportedCodecs,
    ) -> Result<Cid, Self::Error>;
    async fn get(&self, address: Cid) -> Result<Option<Vec<u8>>, Self::Error>;
    async fn delete(&self, address: Cid) -> Result<(), Self::Error>;
}

#[rocket::async_trait]
impl ContentAddressedStorage for Ipfs<DefaultParams> {
    type Error = anyhow::Error;
    async fn put<C: AsyncRead + Send>(
        &self,
        content: C,
        codec: SupportedCodecs,
    ) -> Result<Cid, Self::Error> {
        todo!()
    }
    async fn get(&self, address: Cid) -> Result<Option<Vec<u8>>, Self::Error> {
        todo!()
    }
    async fn delete(&self, address: Cid) -> Result<(), Self::Error> {
        todo!()
    }
}
