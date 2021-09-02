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
    use ipfs_embed::{generate_keypair, Config, Event as SwarmEvent, Ipfs};
    use rocket::futures::StreamExt;
    use std::collections::BTreeMap;

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

        let json = r#"{"hello":"there"}"#;
        let key1 = "my_json.json";
        let key2 = "my_dup_json.json";
        let md: BTreeMap<String, String> =
            [("content-type".to_string(), "application/json".to_string())]
                .to_vec()
                .into_iter()
                .collect();

        let s3_obj_1 = S3ObjectBuilder::new(key1.as_bytes().to_vec(), md.clone());
        let s3_obj_2 = S3ObjectBuilder::new(key2.as_bytes().to_vec(), md.clone());

        alice_service.write(vec![(s3_obj_1, json.as_bytes().to_vec())], vec![])?;
        bob_service.write(vec![(s3_obj_2, json.as_bytes().to_vec())], vec![])?;

        {
            // ensure only alice has s3_obj_1
            let o = alice_service
                .get(key1)?
                .expect("object 1 not found for alice");
            assert_eq!(&o.key, key1.as_bytes());
            assert_eq!(&o.metadata, &md);
            assert_eq!(bob_service.get(key1)?, None);
        };
        {
            // ensure only bob has s3_obj_2
            let o = bob_service.get(key2)?.expect("object 2 not found for bob");
            assert_eq!(&o.key, key2.as_bytes());
            assert_eq!(&o.metadata, &md);
            assert_eq!(alice_service.get(key2)?, None);
        };

        std::thread::sleep_ms(500);
        assert_eq!(
            bob_service.get(key1)?.expect("object 1 not found for bob"),
            alice_service
                .get(key1)?
                .expect("object 1 not found for alice")
        );
        assert_eq!(
            bob_service.get(key2)?.expect("object 2 not found for bob"),
            alice_service
                .get(key2)?
                .expect("object 2 not found for alice")
        );

        // remove key1
        alice_service.write(vec![], vec![(key1.as_bytes().to_vec(), None)])?;

        assert_eq!(alice_service.get(key1)?, None);

        std::thread::sleep_ms(500);

        assert_eq!(bob_service.get(key1)?, None);

        Ok(())
    }
}
