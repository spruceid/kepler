use kepler_lib::{
    libipld::cid::multihash::{Blake3_256, Hasher},
    resource::OrbitId,
};
use libp2p::identity::{
    ed25519::{Keypair as EdKP, SecretKey},
    DecodingError,
};
use sea_orm_migration::async_trait::async_trait;
use std::error::Error as StdError;

pub use libp2p::{
    identity::{Keypair, PublicKey},
    PeerId,
};

pub(crate) fn get_did_key(key: PublicKey) -> String {
    use kepler_lib::libipld::cid::multibase;
    // only ed25519 feature is enabled, so this unwrap should never fail
    let ed25519_pk_bytes = key.try_into_ed25519().unwrap().to_bytes();
    let multicodec_pk = [[0xed].as_slice(), ed25519_pk_bytes.as_slice()].concat();
    format!(
        "did:key:{}",
        multibase::encode(multibase::Base::Base58Btc, multicodec_pk)
    )
}

#[async_trait]
pub trait Secrets {
    type Error: StdError;
    async fn get_keypair(&self, orbit: &OrbitId) -> Result<Keypair, Self::Error>;
    async fn get_pubkey(&self, orbit: &OrbitId) -> Result<PublicKey, Self::Error> {
        Ok(self.get_keypair(orbit).await?.public())
    }
    async fn stage_keypair(&self, orbit: &OrbitId) -> Result<PublicKey, Self::Error>;
    async fn save_keypair(&self, orbit: &OrbitId) -> Result<(), Self::Error>;
    async fn get_peer_id(&self, orbit: &OrbitId) -> Result<PeerId, Self::Error> {
        Ok(self.get_pubkey(orbit).await?.to_peer_id())
    }
}

#[async_trait]
pub trait SecretsSetup {
    type Error: StdError;
    type Input;
    type Output: Secrets;
    async fn setup(&self, input: Self::Input) -> Result<Self::Output, Self::Error>;
}

#[derive(Clone)]
pub struct StaticSecret {
    secret: Vec<u8>,
}

impl StaticSecret {
    pub fn new(secret: Vec<u8>) -> Result<Self, Vec<u8>> {
        if secret.len() < 32 {
            Err(secret)
        } else {
            Ok(Self { secret })
        }
    }
}

#[async_trait]
impl Secrets for StaticSecret {
    type Error = DecodingError;
    async fn get_keypair(&self, orbit: &OrbitId) -> Result<Keypair, Self::Error> {
        let mut hasher = Blake3_256::default();
        hasher.update(&self.secret);
        hasher.update(orbit.to_string().as_bytes());
        let derived = hasher.finalize().to_vec();
        Ok(EdKP::from(SecretKey::try_from_bytes(derived)?).into())
    }
    async fn stage_keypair(&self, orbit: &OrbitId) -> Result<PublicKey, Self::Error> {
        self.get_pubkey(orbit).await
    }
    async fn save_keypair(&self, _orbit: &OrbitId) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[async_trait]
impl SecretsSetup for StaticSecret {
    type Error = std::convert::Infallible;
    type Input = ();
    type Output = Self;
    async fn setup(&self, _input: Self::Input) -> Result<Self::Output, Self::Error> {
        Ok(self.clone())
    }
}
