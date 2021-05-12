use super::{auth::AuthorizationPolicy, cas::ContentAddressedStorage, codec::SupportedCodecs};
use anyhow::{anyhow, Result};
use ipfs_embed::{Config, Ipfs};
use libipld::{
    cid::{
        multibase::Base,
        multihash::{Code, MultihashDigest},
        Cid,
    },
    store::DefaultParams,
};
use libp2p_core::PeerId;
use rocket::futures::stream::StreamExt;
use ssi::did::DIDURL;
use std::{convert::TryFrom, path::Path};

#[rocket::async_trait]
pub trait Orbit: ContentAddressedStorage {
    type Error;
    type UpdateMessage;
    type Auth: AuthorizationPolicy;

    fn id(&self) -> &Cid;

    fn hosts(&self) -> Vec<PeerId>;

    fn admins(&self) -> &[&DIDURL];

    fn auth(&self) -> &Self::Auth;

    fn make_uri(&self, cid: &Cid) -> Result<String, <Self as Orbit>::Error>;

    async fn update(
        &self,
        update: Self::UpdateMessage,
    ) -> Result<(), <Self as ContentAddressedStorage>::Error>;
}

pub struct SimpleOrbit<A: AuthorizationPolicy + Send + Sync> {
    ipfs: Ipfs<DefaultParams>,
    oid: Cid,
    policy: A,
}

pub async fn create_orbit<P, A>(oid: Cid, path: P, policy: A) -> Result<SimpleOrbit<A>>
where
    A: AuthorizationPolicy + Send + Sync,
    P: AsRef<Path>,
{
    let mut cfg = Config::new(
        Some(path.as_ref().join(oid.to_string_of_base(Base::Base58Btc)?)),
        0,
    );

    // TODO enable dht once orbits are defined
    cfg.network.kad = None;
    let ipfs = Ipfs::<DefaultParams>::new(cfg).await?;
    ipfs.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?
        .next()
        .await
        .ok_or_else(|| anyhow!("IPFS Listening Failed"))?;

    Ok(SimpleOrbit { ipfs, oid, policy })
}

pub fn verify_oid_v0(oid: &Cid, pkh: &str, salt: &str) -> Result<()> {
    if &Code::try_from(oid.hash().code())?.digest(format!("{}:{}", salt, pkh).as_bytes())
        == oid.hash()
    {
        Ok(())
    } else {
        Err(anyhow!("Failed to verify Orbit ID"))
    }
}

#[rocket::async_trait]
impl<A> ContentAddressedStorage for SimpleOrbit<A>
where
    A: AuthorizationPolicy + Send + Sync,
{
    type Error = anyhow::Error;
    async fn put(
        &self,
        content: &[u8],
        codec: SupportedCodecs,
    ) -> Result<Cid, <Self as ContentAddressedStorage>::Error> {
        self.ipfs.put(content, codec).await
    }
    async fn get(
        &self,
        address: &Cid,
    ) -> Result<Option<Vec<u8>>, <Self as ContentAddressedStorage>::Error> {
        ContentAddressedStorage::get(&self.ipfs, address).await
    }
    async fn delete(&self, address: &Cid) -> Result<(), <Self as ContentAddressedStorage>::Error> {
        self.delete(address).await
    }
}

#[rocket::async_trait]
impl<A> Orbit for SimpleOrbit<A>
where
    A: AuthorizationPolicy + Send + Sync,
{
    type Error = anyhow::Error;
    type UpdateMessage = ();
    type Auth = A;

    fn id(&self) -> &Cid {
        &self.oid
    }

    fn hosts(&self) -> Vec<PeerId> {
        vec![self.ipfs.local_peer_id()]
    }

    fn admins(&self) -> &[&DIDURL] {
        todo!()
    }

    fn auth(&self) -> &Self::Auth {
        &self.policy
    }

    fn make_uri(&self, cid: &Cid) -> Result<String, <Self as Orbit>::Error> {
        Ok(format!(
            "kepler://v0:{}/{}",
            self.id().to_string_of_base(Base::Base58Btc)?,
            cid.to_string_of_base(Base::Base58Btc)?
        ))
    }

    async fn update(&self, _update: Self::UpdateMessage) -> Result<(), <Self as Orbit>::Error> {
        todo!()
    }
}
