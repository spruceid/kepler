use anyhow::Result;
use ipfs_embed::{Multiaddr, PeerId};
use reqwest;
use serde::Deserialize;
use std::{collections::HashMap as Map, convert::TryFrom, str::FromStr};

#[derive(Default, Debug)]
pub struct Manifest {
    admins: Vec<String>,
    hosts: Map<PeerId, Vec<Multiaddr>>,
}

#[derive(Deserialize)]
pub struct Storage {
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

#[derive(Deserialize)]
#[serde(try_from = "&str")]
struct PID(pub PeerId);

impl TryFrom<&str> for PID {
    type Error = <PeerId as FromStr>::Err;
    fn try_from(v: &str) -> Result<Self, Self::Error> {
        Ok(Self(PeerId::from_str(v)?))
    }
}

const DEFAULT_TZKT_API: &str = "http://localhost:5000";

async fn get_orbit_state(tzkt_api: &str, address: &str) -> Result<Manifest> {
    let storage_url = format!("{}/v1/contracts/{}/storage", tzkt_api, address);
    let storage = reqwest::get(&storage_url).await?.json::<Storage>().await?;

    Ok(Manifest {
        admins: reqwest::get(format!("{}/v1/bigmaps/{}/keys", tzkt_api, storage.admins))
            .await?
            .json::<Vec<BigmapKey>>()
            .await?
            .into_iter()
            .filter_map(|k| if k.active { Some(k.key) } else { None })
            .collect(),
        hosts: reqwest::get(format!("{}/v1/bigmaps/{}/keys", tzkt_api, storage.hosts))
            .await?
            .json::<Vec<BigmapKey<PID, Vec<Multiaddr>>>>()
            .await?
            .into_iter()
            .filter_map(|k| if k.active { Some(k) } else { None })
            .fold(Map::new(), |mut acc, k| {
                acc.insert(k.key.0, k.value);
                acc
            }),
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
