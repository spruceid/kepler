use crate::orbit::{OrbitMetadata, PID};
use anyhow::Result;
use ipfs_embed::{Block, Multiaddr, PeerId};
use libipld::{cid::multihash::Code, raw::RawCodec, store::DefaultParams};
use reqwest;
use serde::{de::DeserializeOwned, Deserialize};
use ssi::did::DIDURL;
use std::{collections::HashMap as Map, convert::TryFrom, str::FromStr};

#[derive(Deserialize)]
struct OrbitStorage {
    admins: u64,
    hosts: u64,
}

#[derive(Debug, Deserialize)]
struct BigmapKey<K = String, V = EmptyObject> {
    active: bool,
    key: K,
    value: V,
}

#[derive(Debug, Deserialize)]
struct EmptyObject {}

const DEFAULT_TZKT_API: &str = "http://localhost:5000";

async fn get_bigmap<K, V>(tzkt_api: &str, bigmap_id: u64) -> Result<impl Iterator<Item = (K, V)>>
where
    K: DeserializeOwned,
    V: DeserializeOwned,
{
    Ok(
        reqwest::get(format!("{}/v1/bigmaps/{}/keys", tzkt_api, bigmap_id))
            .await?
            .json::<Vec<BigmapKey<K, V>>>()
            .await?
            .into_iter()
            .filter_map(|k| {
                if k.active {
                    Some((k.key, k.value))
                } else {
                    None
                }
            }),
    )
}

async fn get_orbit_state(tzkt_api: &str, address: &str) -> Result<OrbitMetadata> {
    let storage_url = format!("{}/v1/contracts/{}/storage", tzkt_api, address);
    let storage = reqwest::get(&storage_url)
        .await?
        .json::<OrbitStorage>()
        .await?;

    Ok(OrbitMetadata {
        id: *Block::<DefaultParams>::encode(
            RawCodec,
            Code::Blake3_256,
            format!("tz;contract={};", address).as_bytes(),
        )?
        .cid(),
        controllers: get_bigmap::<String, EmptyObject>(tzkt_api, storage.admins)
            .await?
            .map(|(k, _)| DIDURL {
                did: format!("did:pkh:tz:{}", k),
                fragment: Some("BlockchainAccountId".into()),
                ..Default::default()
            })
            .collect(),
        hosts: get_bigmap::<PID, Vec<Multiaddr>>(tzkt_api, storage.hosts)
            .await?
            .fold(Map::new(), |mut acc, (k, v)| {
                acc.insert(k, v);
                acc
            }),
        read_delegators: vec![],
        write_delegators: vec![],
        revocations: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    async fn test() -> Result<()> {
        let address = "KT1L7hDDhXynuMoyRGoQNBNUHZEM5iBRu24U";
        let m = get_orbit_state(DEFAULT_TZKT_API, address).await?;
        println!("{:#?}", m);
        assert!(false);
        Ok(())
    }
}
