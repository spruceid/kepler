use anyhow::Error;
use aws_sdk_s3::{
    error::{GetObjectError, GetObjectErrorKind},
    types::{ByteStream, SdkError},
    Client, // Config,
    Error as S3Error,
};
use aws_smithy_http::{body::SdkBody, byte_stream::Error as ByteStreamError, endpoint::Endpoint};
use futures::stream::{IntoAsyncRead, MapErr, TryStreamExt};
use kepler_lib::{
    libipld::cid::{
        multibase::{encode, Base},
        multihash::Multihash,
        Cid,
    },
    resource::OrbitId,
};
use regex::Regex;
use rocket::{async_trait, http::hyper::Uri};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use std::{io::Error as IoError, path::PathBuf, str::FromStr};

use crate::{
    config,
    storage::{dynamodb::DynamoPinStore, ImmutableStore, StorageConfig},
};

#[derive(Debug)]
pub struct S3DataStore {
    // TODO Remove is unused (orbit::delete is never called).
    // When that changes we will need to use a mutex, either local or in Dynamo
    pub client: Client,
    pub bucket: String,
    pub dynamodb: DynamoPinStore,
    pub orbit: Cid,
}

// TODO we could use the same struct for both the block store and the data
// (pin) store, but we need to remember that for now it will be two different
// objects in rust-ipfs
#[derive(Debug, Clone)]
pub struct S3BlockStore {
    // TODO Remove is unused (orbit::delete is never called).
    // When that changes we will need to use a mutex, either local or in Dynamo
    pub client: Client,
    pub bucket: String,
    pub orbit: String,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct S3BlockConfig {
    pub bucket: String,
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[serde(default)]
    pub endpoint: Option<Uri>,
    // pub dynamodb: DynamoStorage,
}

#[async_trait]
impl StorageConfig<S3BlockStore> for S3BlockConfig {
    type Error = std::convert::Infallible;
    async fn open(&self, orbit: &OrbitId) -> Result<S3BlockStore, Self::Error> {
        Ok(S3BlockStore::new_(self, orbit.get_cid()))
    }
}

pub fn new_client(config: &S3BlockConfig) -> Client {
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
    pub fn new_(config: S3BlockConfig, orbit: Cid) -> Self {
        S3DataStore {
            client: new_client(&config),
            bucket: config.bucket,
            dynamodb: todo!(),
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
    pub fn new_(config: &S3BlockConfig, orbit: Cid) -> Self {
        S3BlockStore {
            client: new_client(config),
            bucket: config.bucket.clone(),
            orbit: orbit.to_string(),
        }
    }
}

pub fn convert(e: ByteStreamError) -> IoError {
    e.into()
}

#[async_trait]
impl ImmutableStore for S3BlockStore {
    type Error = S3Error;
    type Readable = IntoAsyncRead<MapErr<ByteStream, fn(ByteStreamError) -> IoError>>;
    async fn contains(&self, id: &Multihash) -> Result<bool, Self::Error> {
        todo!()
    }
    async fn write(
        &self,
        data: impl futures::io::AsyncRead + Send,
    ) -> Result<Multihash, Self::Error> {
        // write to a dummy ID (in fs or in s3)
        // get the hash
        // write or copy to correct ID (hash)
        todo!();
        // write into tmp then rename, to name after the hash
        // need to stream data through a hasher into the file and return hash
        // match File::open(path.join(cid.to_string())),await {
        //     Ok(f) => copy(data, file).await
        //     Err(e) if error.kind() == IoErrorKind::NotFound => Ok(None),
        //     Err(e) => Err(e),
        // }
    }
    async fn remove(&self, id: &Multihash) -> Result<Option<()>, Self::Error> {
        todo!()
    }
    async fn read(&self, id: &Multihash) -> Result<Option<Self::Readable>, Self::Error> {
        let res = self
            .client
            .get_object()
            .bucket(self.bucket.clone())
            .key(format!(
                "{}/{}",
                self.orbit,
                encode(Base::Base64Url, &id.to_bytes())
            ))
            .send()
            .await;
        match res {
            Ok(o) => Ok(Some(
                o.body
                    .map_err(convert as fn(ByteStreamError) -> IoError)
                    .into_async_read(),
            )),
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
