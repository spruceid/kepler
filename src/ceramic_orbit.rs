use crate::orbit::OrbitMetadata;
use anyhow::Result;
use ipfs_embed::{Multiaddr, PeerId};
use libipld::cid::Cid;
use reqwest;
use serde::Deserialize;
use serde_with::{serde_as, DisplayFromStr};
use ssi::did::DIDURL;
use std::collections::HashMap as Map;

#[serde_as]
#[derive(Deserialize)]
struct TileContent {
    #[serde_as(as = "Map<DisplayFromStr, _>")]
    pub hosts: Map<PeerId, Vec<Multiaddr>>,
}

#[derive(Deserialize)]
struct TileMetadata {
    pub controllers: Vec<DIDURL>,
}

#[derive(Deserialize)]
struct TileState {
    pub content: TileContent,
    pub metadata: TileMetadata,
}

#[derive(Deserialize)]
struct StateResponseBody {
    pub state: TileState,
}

async fn get_tile_state(ceramic_api: &str, stream_id: &str) -> Result<TileState> {
    Ok(
        reqwest::get(format!("{}/api/v0/streams/{}", ceramic_api, stream_id))
            .await?
            .json::<StateResponseBody>()
            .await?
            .state,
    )
}

fn didkey_to_did_vm(did: DIDURL) -> DIDURL {
    DIDURL {
        fragment: did
            .fragment
            .or(did.did.strip_prefix("did:key:").map(String::from)),
        ..did
    }
}

pub async fn get_orbit_state(ceramic_api: &str, stream_id: &str, id: Cid) -> Result<OrbitMetadata> {
    let state = get_tile_state(ceramic_api, stream_id).await?;

    Ok(OrbitMetadata {
        id,
        controllers: state
            .metadata
            .controllers
            .into_iter()
            .map(|did| didkey_to_did_vm(did))
            .collect(),
        hosts: state.content.hosts,
        read_delegators: vec![],
        write_delegators: vec![],
        revocations: vec![],
    })
}

pub async fn params_to_ceramic_orbit(
    oid: Cid,
    params: &Map<String, String>,
    ceramic_api: &str,
) -> Result<OrbitMetadata> {
    match params.get("stream") {
        // try read orbit state from stream
        Some(v) => Ok(get_orbit_state(ceramic_api, v, oid).await?),
        _ => Err(anyhow!("Missing stream ID")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    async fn test() -> Result<()> {
        let stream_id = "kjzl6cwe1jw14966o3mfhkccx6rmqjlnx9g0o41hehcpbt7pgx5xc96nub5qxj6";
        let oid = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".parse()?;
        let st = get_orbit_state("http://localhost:7007", stream_id, oid).await?;
        println!("{:#?}", st);
        assert!(false);
        Ok(())
    }
}
