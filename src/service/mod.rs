use rocket::async_trait;

pub mod name;
pub mod s3;

#[async_trait]
pub trait KeplerService {
    type Error;
    type Stopped;

    async fn start(config: Self::Stopped) -> Result<Self, Self::Error>
    where
        Self: Sized;
    async fn stop(self);
}

mod vec_cid_bin {
    use libipld::cid::Cid;
    use serde::{de::Error as DeError, ser::SerializeSeq, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(vec: &Vec<Cid>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = ser.serialize_seq(Some(vec.len()))?;
        for cid in vec {
            seq.serialize_element(&cid.to_bytes())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D>(deser: D) -> Result<Vec<Cid>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Vec<&[u8]> = Deserialize::deserialize(deser)?;
        s.iter()
            .map(|&sc| Cid::read_bytes(sc).map_err(D::Error::custom))
            .collect()
    }
}

#[cfg(test)]
mod test {
    use super::name::*;
    use super::s3::*;
    use super::*;
    use ipfs_embed::{generate_keypair, Block, Config, Event as SwarmEvent, Ipfs};
    use libipld::{multihash::Code, raw::RawCodec, DefaultParams};
    use rocket::futures::StreamExt;
    fn tracing_try_init() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init()
            .ok();
    }

    async fn create_store(id: &str, path: std::path::PathBuf) -> Result<KNSStore, anyhow::Error> {
        std::fs::create_dir_all(&path)?;
        let mut config = Config::new(&path, generate_keypair());
        config.network.broadcast = None;
        let ipfs = Ipfs::new(config).await?;
        ipfs.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?.next().await;
        let task_ipfs = ipfs.clone();
        tokio::spawn(async move {
            let mut events = task_ipfs.swarm_events();
            loop {
                match events.next().await {
                    Some(SwarmEvent::Discovered(p)) => {
                        tracing::debug!("dialing peer {}", p);
                        &task_ipfs.dial(&p);
                    }
                    None => return,
                    _ => continue,
                }
            }
        });
        KNSStore::new(id.to_string(), ipfs, sled::open(path.join("db.sled"))?)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test() -> Result<(), anyhow::Error> {
        tracing_try_init();
        let tmp = tempdir::TempDir::new("test_streams")?;
        let id = "test_id".to_string();

        let alice = create_store(&id, tmp.path().join("alice")).await?;
        let bob = create_store(&id, tmp.path().join("bob")).await?;

        let alice_service = KeplerNameService::start(alice).await?;
        let bob_service = KeplerNameService::start(bob).await?;
        std::thread::sleep_ms(2000);

        let json = r#"{"hello":"there"}"#.as_bytes().to_vec();

        let s3_obj = S3ObjectBuilder::new(
            "my_json.json".as_bytes().to_vec(),
            vec![("content-type".to_string(), "application/json".to_string())],
        );

        alice_service.write(vec![(s3_obj, json)], vec![])?;

        std::thread::sleep_ms(2000);

        assert!(false);
        Ok(())
    }
}
