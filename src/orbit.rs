use crate::{
    auth::{Action, AuthorizationPolicy, AuthorizationToken},
    cas::ContentAddressedStorage,
    codec::SupportedCodecs,
    tz::{TezosAuthorizationString, TezosBasicAuthorization},
};
use anyhow::{anyhow, Result};
use ipfs_embed::{Config, Ipfs, PeerId};
use libipld::{
    cid::{
        multibase::Base,
        multihash::{Code, MultihashDigest},
        Cid,
    },
    store::DefaultParams,
};
use rocket::tokio::fs;

use cached::proc_macro::cached;
use serde::{Deserialize, Serialize};
use ssi::did::DIDURL;
use std::{convert::TryFrom, path::PathBuf};

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

// TODO I think this will need to go, using a trait for a core object creates
// too much monomorphisation which shouldn't be too much of a problem for the
// binary size but I'm not sure how Rocket handles it.
// Or maybe implement the traits separately.
// Ultimately I think we'll need to use an enum, it's not as fancy but because
// we only have simple relationships (e.g. for the Tezos policy you need a
// Tezos token) it will make the routing simpler and easier to use.
#[rocket::async_trait]
pub trait Orbit: ContentAddressedStorage {
    // + AuthorizationPolicy {
    fn id(&self) -> &Cid;

    fn hosts(&self) -> Vec<PeerId>;

    fn admins(&self) -> &[&DIDURL];

    fn auth(&self) -> &AuthMethods;

    fn make_uri(&self, cid: &Cid) -> Result<String>;

    // async fn update(
    //     &self,
    //     update: Self::UpdateMessage,
    // ) -> Result<(), <Self as ContentAddressedStorage>::Error>;
}

#[derive(Clone)]
pub enum AuthMethods {
    Tezos(TezosBasicAuthorization),
}

#[derive(Clone)]
pub enum AuthTokens {
    Tezos(TezosAuthorizationString),
}

impl AuthorizationToken for AuthTokens {
    fn extract(auth_data: &str) -> Result<Self> {
        Err(anyhow!("todo"))
    }

    fn action(&self) -> &Action {
        match self {
            Self::Tezos(token) => token.action(),
        }
    }
}

impl AuthMethods {
    pub async fn authorize(&self, auth_token: AuthTokens) -> Result<()> {
        match self {
            Self::Tezos(method) => match auth_token {
                AuthTokens::Tezos(token) => method.authorize(&token).await,
            },
        }
    }
}

#[derive(Clone)]
pub struct SimpleOrbit {
    ipfs: Ipfs<DefaultParams>,
    oid: Cid,
    policy: AuthMethods,
}

pub async fn create_orbit(
    oid: Cid,
    path: PathBuf,
    _policy: TezosBasicAuthorization,
    controllers: Vec<DIDURL>,
    // TODO put back access logs
    // auth: &[u8],
) -> Result<SimpleOrbit> {
    let dir = path.join(oid.to_string_of_base(Base::Base58Btc)?);

    // fails if DIR exists, this is Create, not Open
    fs::create_dir(&dir)
        .await
        .map_err(|_| anyhow!("Orbit already exists"))?;

    // create default and write
    let md = OrbitMetadata {
        id: oid.clone(),
        controllers,
        read_delegators: vec![],
        write_delegators: vec![],
        revocations: vec![],
    };
    fs::write(dir.join("metadata"), serde_json::to_vec_pretty(&md)?).await?;
    // fs::write(dir.join("access_log"), auth).await?;

    // Ok(load_orbit(oid, path)
    //     .await
    //     .map(|o| o.ok_or_else(|| anyhow!("Dir not created")))??)
    load_orbit(oid, path).await
}

// 100 orbits => 600 FDs
// 1min timeout to evict orbits that might have been deleted
#[cached(size = 100, time = 60, result = true)]
pub async fn load_orbit(oid: Cid, path: PathBuf) -> Result<SimpleOrbit> {
    let dir = path.join(oid.to_string_of_base(Base::Base58Btc)?);
    // if !dir.exists() {
    //     return Ok(None);
    // }
    let mut cfg = Config::new(Some(dir.join("block_store")), 0);
    cfg.network.mdns = None;
    cfg.network.gossipsub = None;
    cfg.network.broadcast = None;
    cfg.network.bitswap = None;

    let md: OrbitMetadata = serde_json::from_slice(&fs::read(dir.join("metadata")).await?)?;

    // TODO enable dht once orbits are defined
    cfg.network.kad = None;
    let ipfs = Ipfs::<DefaultParams>::new(cfg).await?;

    Ok(SimpleOrbit {
        ipfs,
        oid: md.id,
        // TODO retrieve policies from the orbit
        policy: AuthMethods::Tezos(TezosBasicAuthorization),
    })
}

pub fn verify_oid(oid: &Cid, pkh: &str, uri_str: &str) -> Result<()> {
    // try to parse as a URI with matrix params
    let mut parts = uri_str.split(';');
    let method = parts.next().ok_or(anyhow!("No URI"))?;

    if &Code::try_from(oid.hash().code())?.digest(uri_str.as_bytes()) == oid.hash()
        && oid.codec() == 0x55
        && match method {
            "tz" => parts
                .map(|part| part.split('=').collect::<Vec<&str>>())
                .filter_map(|kv| {
                    if kv.len() == 2 {
                        Some((kv[0], kv[1]))
                    } else {
                        None
                    }
                })
                .any(|(name, value)| name == "address" && value == pkh),
            _ => Err(anyhow!("Orbit method not registered"))?,
        }
    {
        Ok(())
    } else {
        Err(anyhow!("Failed to verify Orbit ID"))
    }
}

#[rocket::async_trait]
impl ContentAddressedStorage for SimpleOrbit {
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
        self.ipfs.delete(address).await
    }
    async fn list(&self) -> Result<Vec<Cid>, <Self as ContentAddressedStorage>::Error> {
        self.ipfs.list().await
    }
}

#[rocket::async_trait]
impl Orbit for SimpleOrbit {
    fn id(&self) -> &Cid {
        &self.oid
    }

    fn hosts(&self) -> Vec<PeerId> {
        vec![self.ipfs.local_peer_id()]
    }

    fn admins(&self) -> &[&DIDURL] {
        todo!()
    }

    fn auth(&self) -> &AuthMethods {
        &self.policy
    }

    fn make_uri(&self, cid: &Cid) -> Result<String> {
        Ok(format!(
            "kepler://{}/{}",
            self.id().to_string_of_base(Base::Base58Btc)?,
            cid.to_string_of_base(Base::Base58Btc)?
        ))
    }

    // async fn update(&self, _update: Self::UpdateMessage) -> Result<(), <Self as Orbit>::Error> {
    //     todo!()
    // }
}

#[test]
async fn oid_verification() {
    let oid: Cid = "zCT5htkeBtA6Qu5YF4vPkQcfeqy3pY4m8zxGdUKUiPgtPEbY3rHy"
        .parse()
        .unwrap();
    let pkh = "tz1YSb7gXhgBw46nSXthhoSzhJdbQf9h92Gy";
    let domain = "kepler.tzprofiles.com";
    let index = 0;
    let uri = format!("tz;address={};domain={};index={}", pkh, domain, index);
    verify_oid(&oid, pkh, &uri).unwrap();
}
