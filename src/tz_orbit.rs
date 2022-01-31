use crate::orbit::OrbitMetadata;
use anyhow::Result;
use libipld::cid::Cid;
use libp2p::{Multiaddr, PeerId};
use reqwest;
use serde::{de::DeserializeOwned, Deserialize};
use ssi::did::DIDURL;
use std::{collections::HashMap as Map, str::FromStr};

#[derive(Deserialize)]
struct OrbitStorage {
    admins: u64,
    hosts: u64,
    readers: u64,
    writers: u64,
}

#[derive(Debug, Deserialize)]
struct BigmapKey<K, V> {
    active: bool,
    key: K,
    value: V,
}

#[derive(Debug, Deserialize)]
struct UnitObject {}

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

fn pkh_to_did_vm(pkh: &str) -> DIDURL {
    DIDURL {
        did: format!("did:pkh:tz:{}", pkh),
        fragment: Some("TezosMethod2021".into()),
        ..Default::default()
    }
}

pub async fn get_orbit_state(tzkt_api: &str, address: &str, id: Cid) -> Result<OrbitMetadata> {
    let storage_url = format!("{}/v1/contracts/{}/storage", tzkt_api, address);
    let storage = reqwest::get(&storage_url)
        .await?
        .json::<OrbitStorage>()
        .await?;

    Ok(OrbitMetadata {
        id,
        controllers: get_bigmap::<String, UnitObject>(tzkt_api, storage.admins)
            .await?
            .map(|(k, _)| pkh_to_did_vm(&k))
            .collect(),
        hosts: get_bigmap::<String, Vec<Multiaddr>>(tzkt_api, storage.hosts)
            .await?
            .map(|(k, v)| Ok((PeerId::from_str(&k)?, v)))
            .collect::<Result<Map<PeerId, Vec<Multiaddr>>>>()?,
    })
}

pub async fn params_to_tz_orbit(
    oid: Cid,
    params: &Map<String, String>,
    tzkt_api: &Option<String>,
) -> Result<OrbitMetadata> {
    match (params.get("address"), params.get("contract"), tzkt_api) {
        // try read orbit state from chain
        (_, Some(v), Some(a)) => Ok(get_orbit_state(a, v, oid).await?),
        // try use implicit address key as controller
        (Some(v), None, _) => Ok(OrbitMetadata {
            id: oid,
            controllers: vec![pkh_to_did_vm(v)],
            hosts: Map::new(),
        }),
        _ => Err(anyhow!("Missing address or contract")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    async fn test() -> Result<()> {
        Ok(())
    }
}
