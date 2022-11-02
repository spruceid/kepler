use anyhow::{Context, Error};
use aws_sdk_dynamodb::{
    error::{GetItemError, GetItemErrorKind},
    model::{AttributeValue, ReturnValue},
    output::ScanOutput,
    types::SdkError,
    Client,
};
use aws_smithy_http::endpoint::Endpoint;
use futures::{
    lock::Mutex,
    stream::{self, StreamExt, TryStreamExt},
};
use kepler_lib::libipld::cid::{multibase::Base, Cid};
use rocket::async_trait;
use std::{collections::BTreeSet, str::FromStr};

use crate::config;

const CID_ATTRIBUTE: &str = "Cid";
const ROOT_ATTRIBUTE: &str = "Root";
const PARENTS_ATTRIBUTE: &str = "Parents";

#[derive(Debug)]
pub struct DynamoPinStore {
    // TODO no need for Mutex???
    client: Mutex<Client>,
    table: String,
    orbit: Cid,
}

impl DynamoPinStore {
    pub fn new(config: config::DynamoStorage, orbit: Cid) -> Self {
        let general_config = super::utils::aws_config();
        let sdk_config = aws_sdk_dynamodb::config::Builder::from(&general_config);
        let sdk_config = match config.endpoint {
            Some(e) => sdk_config.endpoint_resolver(Endpoint::immutable(e)),
            None => sdk_config,
        };
        let sdk_config = sdk_config.build();
        let client = Mutex::new(Client::from_conf(sdk_config));
        Self {
            client,
            table: config.table,
            orbit,
        }
    }

    pub async fn healthcheck(&self) -> Result<(), Error> {
        // TODO ideally that would be in the builder
        self.client
            .lock()
            .await
            .describe_table()
            .table_name(self.table.clone().clone())
            .send()
            .await
            .context(anyhow!("Failed healthchec for table `{}`", self.table))?;
        Ok(())
    }
}
