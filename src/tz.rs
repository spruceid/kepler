use anyhow::Result;
use cid::Cid;
use did_tezos::DIDTz;
use ssi::{jwk::Algorithm, jws::verify_bytes, ldp::resolve_key};
use tokio;

pub struct TZAuth {
    pub data: String,
    pub sig: String,
    pub pkh: String,
    pub timestamp: String,
    pub orbit: String,
    pub action: String,
    pub cid: Cid,
}

#[tokio::main]
pub async fn verify(auth: TZAuth) -> Result<()> {
    Ok(verify_bytes(
        Algorithm::EdDSA,
        auth.data.as_bytes(),
        &resolve_key(
            &format!("did:tz:{}#blockchainAccountId", &auth.pkh),
            &DIDTz::default(),
        )
        .await?,
        auth.sig.as_bytes(),
    )?)
}
