use anyhow::Result;
use reqwest;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap as Map;

#[derive(Default, Debug)]
pub struct Manifest {
    admins: Vec<String>,
    hosts: Map<String, Vec<String>>,
}

#[derive(Deserialize)]
pub struct Storage {
    admins: u64,
    hosts: u64,
}

#[derive(Debug, Deserialize)]
struct BigmapKey<V> {
    active: bool,
    key: String,
    value: V,
}

#[derive(Debug, Deserialize)]
struct EmptyObject {}

const DEFAULT_TZKT_API: &str = "http://localhost:5000";

async fn get_orbit_state(address: &str) -> Result<Manifest> {
    let storage_url = format!("{}/v1/contracts/{}/storage", DEFAULT_TZKT_API, address);
    let storage = reqwest::get(&storage_url).await?.json::<Storage>().await?;

    Ok(Manifest {
        admins: reqwest::get(format!(
            "{}/v1/bigmaps/{}/keys",
            DEFAULT_TZKT_API, storage.admins
        ))
        .await?
        .json::<Vec<BigmapKey<EmptyObject>>>()
        .await?
        .into_iter()
        .filter_map(|k| if k.active { Some(k.key) } else { None })
        .collect(),
        hosts: reqwest::get(format!(
            "{}/v1/bigmaps/{}/keys",
            DEFAULT_TZKT_API, storage.hosts
        ))
        .await?
        .json::<Vec<BigmapKey<Vec<String>>>>()
        .await?
        .into_iter()
        .filter_map(|k| if k.active { Some(k) } else { None })
        .fold(Map::new(), |mut acc, k| {
            if k.active {
                acc.insert(k.key, k.value);
            };
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
        let m = get_orbit_state(address).await?;
        println!("{:#?}", m);
        assert!(false);
        Ok(())
    }
}
