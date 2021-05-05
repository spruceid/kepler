use super::{
    auth::AuthorizationPolicy, cas::ContentAddressedStorage, codec::SupportedCodecs, CidWrap,
    Orbits,
};
use anyhow::{anyhow, Result};
use ipfs_embed::{Config, Ipfs};
use libipld::{
    cid::{multibase::Base, Cid},
    store::DefaultParams,
};
use libp2p_core::PeerId;
use rocket::{
    futures::stream::StreamExt,
    http::Status,
    request::{FromRequest, Outcome, Request},
    tokio::io::AsyncRead,
};
use ssi::did::DIDURL;
use std::path::Path;

#[rocket::async_trait]
pub trait Orbit: ContentAddressedStorage {
    type Error;
    type UpdateMessage;
    type Auth: AuthorizationPolicy;

    fn id(&self) -> &Cid;

    fn hosts(&self) -> Vec<PeerId>;

    fn admins(&self) -> &[&DIDURL];

    fn auth(&self) -> &Self::Auth;

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
        Some(path.as_ref().join(oid.to_string_of_base(Base::Base64Url)?)),
        0,
    );

    // TODO enable dht once orbits are defined
    cfg.network.kad = None;
    let ipfs = Ipfs::<DefaultParams>::new(cfg).await?;
    ipfs.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?
        .next()
        .await
        .ok_or(anyhow!("IPFS Listening Failed"))?;

    Ok(SimpleOrbit { ipfs, oid, policy })
}

#[rocket::async_trait]
impl<A> ContentAddressedStorage for SimpleOrbit<A>
where
    A: AuthorizationPolicy + Send + Sync,
{
    type Error = anyhow::Error;
    async fn put<C: AsyncRead + Send + Unpin>(
        &self,
        content: &mut C,
        codec: SupportedCodecs,
    ) -> Result<Cid, <Self as ContentAddressedStorage>::Error> {
        self.ipfs.put(content, codec).await
    }
    async fn get(
        &self,
        address: &Cid,
    ) -> Result<Option<Vec<u8>>, <Self as ContentAddressedStorage>::Error> {
        self.get(address).await
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

    async fn update(&self, update: Self::UpdateMessage) -> Result<(), <Self as Orbit>::Error> {
        todo!()
    }
}

#[rocket::async_trait]
impl<'r, A> FromRequest<'r> for &'r SimpleOrbit<A>
where
    A: 'static + AuthorizationPolicy + Send + Sync,
{
    type Error = anyhow::Error;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match req.param::<CidWrap>(0) {
            Some(Ok(oid)) => match req.rocket().state::<Orbits<SimpleOrbit<A>>>() {
                Some(orbits) => match orbits.orbits().get(&oid.0) {
                    Some(orbit) => Outcome::Success(orbit),
                    None => Outcome::Failure((Status::NotFound, anyhow!("No Orbit"))),
                },
                // TODO check filesystem and init/cache if unused orbit db found
                None => Outcome::Failure((Status::NotFound, anyhow!("No Orbit"))),
            },
            Some(Err(e)) => Outcome::Failure((Status::NotFound, e)),
            None => Outcome::Failure((Status::NotFound, anyhow!("No Orbit"))),
        }
    }
}
