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
use rocket::{futures::stream::StreamExt, http::uri::Absolute, tokio::fs};
use serde::{Deserialize, Serialize};
use ssi::did::DIDURL;
use std::{convert::TryFrom, path::Path};

#[derive(Serialize, Deserialize)]
pub struct OrbitMetadata {
    // NOTE This will always serialize in b58check
    #[serde(with = "cid_serde")]
    pub id: Cid,
    pub controllers: Vec<DIDURL>,
    pub read_delegators: Vec<DIDURL>,
    pub write_delegators: Vec<DIDURL>,
    // TODO placeholder type
    pub revocations: Vec<String>,
}

mod cid_serde {
    use libipld::cid::{multibase::Base, Cid};
    use serde::{de::Error as SError, ser::Error as DError, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(cid: &Cid, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ser.serialize_str(
            &cid.to_string_of_base(Base::Base58Btc)
                .map_err(S::Error::custom)?,
        )
    }

    pub fn deserialize<'de, D>(deser: D) -> Result<Cid, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &str = Deserialize::deserialize(deser)?;
        s.parse().map_err(D::Error::custom)
    }
}

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

pub async fn create_orbit<P, A>(
    oid: Cid,
    path: P,
    policy: A,
    controllers: Vec<DIDURL>,
    auth: &[u8],
) -> Result<SimpleOrbit<A>>
where
    A: AuthorizationPolicy + Send + Sync,
    P: AsRef<Path>,
{
    let dir = path.as_ref().join(oid.to_string_of_base(Base::Base58Btc)?);

    // fails if DIR exists, this is Create, not Open
    fs::create_dir(&dir)
        .await
        .map_err(|_| anyhow!("Orbit already exists"))?;

    let mut cfg = Config::new(Some(dir.join("block_store")), 0);

    // create default and write
    let md = OrbitMetadata {
        id: oid.clone(),
        controllers,
        read_delegators: vec![],
        write_delegators: vec![],
        revocations: vec![],
    };

    fs::write(dir.join("metadata"), serde_json::to_vec_pretty(&md)?).await?;

    fs::write(dir.join("access_log"), auth).await?;

    // TODO enable dht once orbits are defined
    cfg.network.kad = None;
    let ipfs = Ipfs::<DefaultParams>::new(cfg).await?;
    ipfs.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?
        .next()
        .await;
    // .ok_or_else(|| anyhow!("IPFS Listening Failed"))?;

    Ok(SimpleOrbit { ipfs, oid, policy })
}

pub async fn load_orbit<P, A>(path: P, policy: A) -> Result<SimpleOrbit<A>>
where
    A: AuthorizationPolicy + Send + Sync,
    P: AsRef<Path>,
{
    let mut cfg = Config::new(Some(path.as_ref().join("block_store")), 0);

    let md: OrbitMetadata =
        serde_json::from_slice(&fs::read(path.as_ref().join("metadata")).await?)?;

    // TODO enable dht once orbits are defined
    cfg.network.kad = None;
    let ipfs = Ipfs::<DefaultParams>::new(cfg).await?;
    ipfs.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?
        .next()
        .await
        .ok_or_else(|| anyhow!("IPFS Listening Failed"))?;

    Ok(SimpleOrbit {
        ipfs,
        oid: md.id,
        policy,
    })
}

pub fn verify_oid_v0(oid: &Cid, pkh: &str, params: &str) -> Result<()> {
    let uri = format!("tz:{}{}", pkh, params);
    // try to parse as a URL with query params
    Absolute::parse(&uri).map_err(|_| anyhow!("Orbit Parameters Invalid"))?;
    if &Code::try_from(oid.hash().code())?.digest(uri.as_bytes()) == oid.hash()
        && oid.codec() == 0x55
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
