use anyhow::{Context, Error};
use aws_sdk_s3::{
    error::{GetObjectError, GetObjectErrorKind},
    types::{ByteStream, SdkError},
    Client, // Config,
};
use aws_smithy_http::{body::SdkBody, endpoint::Endpoint};
// use aws_types::{credentials::Credentials, region::Region};
use futures::stream::{StreamExt, TryStreamExt};
use ipfs::{
    repo::{
        BlockPut, BlockRm, BlockRmError, BlockStore, Column, DataStore, PinKind, PinMode, PinStore,
    },
    Block,
};
use kepler_lib::libipld::cid::{multibase::Base, Cid};
use regex::Regex;
use rocket::{async_trait, http::hyper::Uri};
use std::{path::PathBuf, str::FromStr};

use super::dynamodb::{DynamoPinStore, References};
use crate::config;

// TODO we could use the same struct for both the block store and the data
// (pin) store, but we need to remember that for now it will be two different
// objects in rust-ipfs
#[derive(Debug)]
pub struct S3BlockStore {
    // TODO Remove is unused (orbit::delete is never called).
    // When that changes we will need to use a mutex, either local or in Dynamo
    pub client: Client,
    pub bucket: String,
    pub orbit: Cid,
}

pub fn new_client(config: config::S3BlockStorage) -> Client {
    let general_config = super::utils::aws_config();
    let sdk_config = aws_sdk_s3::config::Builder::from(&general_config);
    let sdk_config = match config.endpoint {
        Some(e) => sdk_config.endpoint_resolver(Endpoint::immutable(e)),
        None => sdk_config,
    };
    let sdk_config = sdk_config.build();
    Client::from_conf(sdk_config)
}

impl S3DataStore {
    pub fn new_(config: config::S3BlockStorage, orbit: Cid) -> Self {
        S3DataStore {
            client: new_client(config.clone()),
            bucket: config.bucket,
            orbit,
        }
    }

    pub async fn get_(&self, key: String) -> Result<Option<Vec<u8>>, Error> {
        let res = self
            .client
            .get_object()
            .bucket(self.bucket.clone())
            .key(format!(
                "{}/{}",
                self.orbit.to_string_of_base(Base::Base58Btc)?,
                key
            ))
            .send()
            .await;
        match res {
            Ok(o) => Ok(Some(o.body.collect().await?.into_bytes().to_vec())),
            Err(SdkError::ServiceError {
                err:
                    GetObjectError {
                        kind: GetObjectErrorKind::NoSuchKey(_),
                        ..
                    },
                ..
            }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn put_(&self, key: String, body: Vec<u8>) -> Result<(), Error> {
        self.client
            .put_object()
            .bucket(self.bucket.clone())
            .key(format!(
                "{}/{}",
                self.orbit.to_string_of_base(Base::Base58Btc)?,
                key
            ))
            .body(ByteStream::new(SdkBody::from(body)))
            .send()
            .await?;
        Ok(())
    }
}
impl S3BlockStore {
    pub fn new_(config: config::S3BlockStorage, orbit: Cid) -> Self {
        S3BlockStore {
            client: new_client(config.clone()),
            bucket: config.bucket,
            orbit,
        }
    }
}

fn path_to_config(path: PathBuf) -> (config::S3BlockStorage, Cid) {
    let re =
            Regex::new(r"^/s3bucket/(?P<bucket>.*)/s3endpoint/(?P<s3endpoint>.*)/dynamotable/(?P<table>.*)/dynamoendpoint/(?P<dynamoendpoint>.*)/orbitcid/(?P<orbit>.*)/(blockstore|datastore)$")
                .unwrap();
    let fields = re.captures(path.to_str().unwrap()).unwrap();
    let s3_bucket = fields.name("bucket").unwrap().as_str().to_string();
    let s3_endpoint = Some(fields.name("s3endpoint").unwrap().as_str())
        .filter(|s| !s.is_empty())
        .map(|e| Uri::from_str(e).unwrap());
    let dynamo_table = fields.name("table").unwrap().as_str().to_string();
    let dynamo_endpoint = Some(fields.name("dynamoendpoint").unwrap().as_str())
        .filter(|s| !s.is_empty())
        .map(|e| Uri::from_str(e).unwrap());
    let orbit = Cid::from_str(fields.name("orbit").unwrap().as_str()).unwrap();

    let config = config::S3BlockStorage {
        bucket: s3_bucket,
        endpoint: s3_endpoint,
        dynamodb: config::DynamoStorage {
            table: dynamo_table,
            endpoint: dynamo_endpoint,
        },
    };
    (config, orbit)
}

#[async_trait]
impl BlockStore for S3BlockStore {
    fn new(path: PathBuf) -> Self {
        let (config, orbit) = path_to_config(path);
        S3BlockStore::new_(config, orbit)
    }

    async fn init(&self) -> Result<(), Error> {
        self.dynamodb
            .healthcheck()
            .await
            .context("Failed healthcheck for DynamoDB")?;
        self.client
            .head_bucket()
            .bucket(self.bucket.clone())
            .send()
            .await
            .context("Failed healthcheck for S3")?;
        Ok(())
    }

    async fn open(&self) -> Result<(), Error> {
        Ok(())
    }

    async fn contains(&self, cid: &Cid) -> Result<bool, Error> {
        self.dynamodb.is_pinned(cid).await
    }

    async fn get(&self, cid: &Cid) -> Result<Option<Block>, Error> {
        let res = self
            .client
            .get_object()
            .bucket(self.bucket.clone())
            .key(format!(
                "{}/{}",
                self.orbit.to_string_of_base(Base::Base58Btc)?,
                cid
            ))
            .send()
            .await;
        match res {
            Ok(o) => Ok(Some(Block::new(
                *cid,
                o.body.collect().await?.into_bytes().to_vec(),
            )?)),
            Err(SdkError::ServiceError {
                err:
                    GetObjectError {
                        kind: GetObjectErrorKind::NoSuchKey(_),
                        ..
                    },
                ..
            }) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn put(&self, block: Block) -> Result<(Cid, BlockPut), Error> {
        let res = self
            .client
            .put_object()
            .bucket(self.bucket.clone())
            .key(format!(
                "{}/{}",
                self.orbit.to_string_of_base(Base::Base58Btc)?,
                block.cid()
            ))
            .body(ByteStream::new(SdkBody::from(block.data())))
            .send()
            .await;

        match res {
            // can't tell if the object already existed
            Ok(_) => Ok((*block.cid(), BlockPut::NewBlock)),
            Err(e) => Err(e.into()),
        }
    }

    async fn remove(&self, cid: &Cid) -> Result<Result<BlockRm, BlockRmError>, Error> {
        // TODO when is that called, should the pin store call this?
        let res = self
            .client
            .delete_object()
            .bucket(self.bucket.clone())
            .key(format!(
                "{}/{}",
                self.orbit.to_string_of_base(Base::Base58Btc)?,
                cid
            ))
            .send()
            .await;

        match res {
            // Cannot tell if the object existed in the first place.
            Ok(_) => Ok(Ok(BlockRm::Removed(*cid))),
            Err(e) => Err(e.into()),
        }
    }

    async fn list(&self) -> Result<Vec<Cid>, Error> {
        self.dynamodb
            .list(None)
            .await
            .map(|r| r.map(|rr| rr.0))
            .try_collect()
            .await
    }

    async fn wipe(&self) {
        unimplemented!("wipe")
    }
}
