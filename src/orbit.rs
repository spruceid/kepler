use anyhow::{anyhow, Error, Result};
use libp2p_core::{PublicKey, PeerId};
use ssi::did::DIDURL;
use super::{codec::SupportedCodecs, cas::ContentAddressedStorage, CidWrap, Orbits};
use ipfs_embed::{Ipfs, Config};
use rocket::{
    http::Status,
    request::{FromRequest, Outcome, Request},
    futures::stream::StreamExt,
    tokio::io::AsyncRead
};
use libipld::{
    block::Block,
    cid::{
        multihash::{Code, MultihashDigest},
        multibase::Base,
        Cid,
    },
    raw::RawCodec,
    store::DefaultParams,
};
use std::path::Path;

#[rocket::async_trait]
pub trait Orbit: ContentAddressedStorage {
    type Error;
    type UpdateMessage;

    fn id(&self) -> &Cid;

    fn hosts(&self) -> Vec<PeerId>;

    fn admins(&self) -> &[&DIDURL];

    async fn update(&self, update: Self::UpdateMessage) -> Result<(), <Self as ContentAddressedStorage>::Error>;
}

pub struct SimpleOrbit {
    ipfs: Ipfs<DefaultParams>,
    oid: Cid
}

pub async fn create_orbit<P: AsRef<Path>>(oid: Cid, path: P) -> Result<SimpleOrbit> {
    let mut cfg = Config::new(
        Some(path.as_ref().join(oid.to_string_of_base(Base::Base64Url)?)),
        0,
    );

    // TODO enable dht once orbits are defined
    cfg.network.kad = None;
    let ipfs = Ipfs::<DefaultParams>::new(cfg).await?;
    ipfs.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?
    .next()
    .await.ok_or(anyhow!("IPFS Listening Failed"))?;

    Ok(SimpleOrbit { ipfs, oid })
}

#[rocket::async_trait]
impl ContentAddressedStorage for SimpleOrbit {
    type Error = anyhow::Error;
    async fn put<C: AsyncRead + Send + Unpin>(
        &self,
        content: &mut C,
        codec: SupportedCodecs,
    ) -> Result<Cid, <Self as ContentAddressedStorage>::Error> {
        self.ipfs.put(content, codec).await
    }
    async fn get(&self, address: &Cid) -> Result<Option<Vec<u8>>, <Self as ContentAddressedStorage>::Error> {
        self.get(address).await
    }
    async fn delete(&self, address: &Cid) -> Result<(), <Self as ContentAddressedStorage>::Error> {
        self.delete(address).await
    }
}

#[rocket::async_trait]
impl Orbit for SimpleOrbit {
    type Error = anyhow::Error;
    type UpdateMessage = ();

    fn id(&self) -> &Cid {
        &self.oid
    }

    fn hosts(&self) -> Vec<PeerId> {
        vec![self.ipfs.local_peer_id()]
    }

    fn admins(&self) -> &[&DIDURL] {
        todo!()
    }

    async fn update(&self, update: Self::UpdateMessage) -> Result<(), <Self as Orbit>::Error> {
        todo!()
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for &'r SimpleOrbit {
    type Error = anyhow::Error;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match req.param::<CidWrap>(0) {
            Some(Ok(oid)) => match req.rocket().state::<Orbits<SimpleOrbit>>() {
                Some(orbits) => match orbits.orbit(&oid.0) {
                    Some(orbit) => Outcome::Success(orbit),
                    None => Outcome::Failure((Status::NotFound, anyhow!("No Orbit")))
                },
                // TODO check filesystem and init/cache if unused orbit db found
                None => Outcome::Failure((Status::NotFound, anyhow!("No Orbit")))
            },
            Some(Err(e)) => Outcome::Failure((Status::NotFound, e)),
            None => Outcome::Failure((Status::NotFound, anyhow!("No Orbit")))
        }
    }
}
