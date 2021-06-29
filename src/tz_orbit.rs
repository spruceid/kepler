use anyhow::Result;
use reqwest;
use serde_json::Value;
use std::collections::HashMap as Map;

#[derive(Default, Debug)]
pub struct Manifest {
    admins: Vec<String>,
    hosts: Map<String, Vec<String>>,
}

const DEFAULT_API: &str = "http://localhost:14000";

async fn get_orbit_state(address: &str) -> Result<Manifest> {
    let url = format!("{}/v1/contract/sandboxnet/{}/storage", DEFAULT_API, address);
    println!("{}", &url);
    let resp = reqwest::get(&url)
        .await?
        .json::<Map<String, Value>>()
        .await?;
    println!("{:#?}", resp);
    Ok(Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    async fn test() -> Result<()> {
        let m = get_orbit_state("KT1EEAkHJEAJhqhQChcMaEuG2t3KdenbsVRh").await?;
        // println!("{:#?}", m);
        assert!(false);
        Ok(())
    }
}
