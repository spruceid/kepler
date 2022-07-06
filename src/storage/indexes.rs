use anyhow::{Context, Result};
use aws_sdk_dynamodb::{
    model::{AttributeValue, ReturnValue},
    output::GetItemOutput,
    types::Blob,
    Client,
};
use aws_smithy_http::endpoint::Endpoint;
use futures::stream::{self, TryStreamExt};
use kepler_lib::libipld::{cid::Cid, multibase::Base};
use sled::{Batch, Tree};
use std::str::FromStr;

use crate::config;

const KEY_ATTRIBUTE: &str = "KVKey";
const VALUE_ATTRIBUTE: &str = "KVValue";
const ELEMENTS_ATTRIBUTE: &str = "KVElements";

#[derive(Clone)]
pub enum KV {
    Sled(Tree),
    DynamoDB(Box<KVDynamoDB>),
}

#[derive(Clone)]
pub struct KVDynamoDB {
    client: Client,
    table: String,
    orbit: String,
    subsystem: String,
    prefix: String,
}

impl KVDynamoDB {
    fn build_key<N: AsRef<[u8]>>(&self, key: N) -> String {
        format!(
            "{}/{}/{}/{}",
            self.orbit,
            self.subsystem,
            self.prefix,
            hex::encode(key)
        )
    }
}

impl KV {
    pub async fn new(
        orbit_id: Cid,
        subsystem_name: String,
        table_name: String,
        config: config::IndexStorage,
    ) -> Result<Self> {
        match config {
            config::IndexStorage::Local(c) => {
                let path = c
                    .path
                    .join(orbit_id.to_string_of_base(Base::Base58Btc)?)
                    .join(subsystem_name)
                    .join(table_name)
                    .join("db.sled");
                tokio::fs::create_dir_all(&path).await?;
                let db = sled::open(path)?;
                let elements = db.open_tree("elements".as_bytes())?;
                Ok(KV::Sled(elements))
            }
            config::IndexStorage::DynamoDB(c) => {
                let general_config = super::utils::aws_config();
                let sdk_config = aws_sdk_dynamodb::config::Builder::from(&general_config);
                let sdk_config = match c.endpoint {
                    Some(e) => sdk_config.endpoint_resolver(Endpoint::immutable(e)),
                    None => sdk_config,
                };
                let sdk_config = sdk_config.build();
                let client = Client::from_conf(sdk_config);
                client
                    .describe_table()
                    .table_name(c.table.clone())
                    .send()
                    .await
                    .context(anyhow!("Failed healthchec for table `{}`", c.table))?;
                Ok(Self::DynamoDB(Box::new(KVDynamoDB {
                    client,
                    table: c.table,
                    orbit: orbit_id.to_string_of_base(Base::Base58Btc)?,
                    subsystem: subsystem_name,
                    prefix: table_name,
                })))
            }
        }
    }

    pub async fn healthcheck(config: config::IndexStorage) -> Result<()> {
        match config.clone() {
            config::IndexStorage::Local(c) => {
                if !c.path.is_dir() {
                    return Err(anyhow!(
                        "KEPLER_STORAGE_INDEXES_PATH does not exist or is not a directory: {:?}",
                        c.path.to_str()
                    ));
                }
            }
            config::IndexStorage::DynamoDB(_) => {
                Self::new(
                    Cid::from_str("bafkreieq5jui4j25lacwomsqgjeswwl3y5zcdrresptwgmfylxo2depppq")
                        .unwrap(),
                    "".to_string(),
                    "".to_string(),
                    config,
                )
                .await
                .context("Failed healthcheck for DynamoDB index storage")?;
            }
        }
        Ok(())
    }

    pub async fn get<N: AsRef<[u8]>>(&self, key: N) -> Result<Option<Vec<u8>>> {
        match self {
            KV::Sled(c) => Ok(c.get(key)?.map(|v| v.to_vec())),
            KV::DynamoDB(c) => {
                let key_ = c.build_key(key);
                match c
                    .client
                    .get_item()
                    .table_name(c.table.clone())
                    .key(KEY_ATTRIBUTE, AttributeValue::S(key_.clone()))
                    // .projection_expression(format!("{}, {}", KEY_ATTRIBUTE, VALUE_ATTRIBUTE))
                    .send()
                    .await
                {
                    Ok(GetItemOutput {
                        item: Some(output), ..
                    }) => match output.get(VALUE_ATTRIBUTE) {
                        Some(AttributeValue::B(blob)) => Ok(Some(blob.clone().into_inner())),
                        Some(_) => Err(anyhow!("Wrong type for value")),
                        None => Err(anyhow!("No `{}` column", KEY_ATTRIBUTE)),
                    },
                    Ok(_) => Ok(None),
                    Err(e) => Err(anyhow!("Error checking on item: {}", e)),
                }
            }
        }
    }

    pub async fn insert<N: AsRef<[u8]>, E: AsRef<[u8]> + ?Sized>(
        &self,
        key: N,
        element: &E,
    ) -> Result<Option<Vec<u8>>> {
        match self {
            KV::Sled(c) => Ok(c
                .insert(key, element.as_ref())
                .map(|e| e.map(|v| v.to_vec()))?),
            KV::DynamoDB(c) => {
                let old = c
                    .client
                    .put_item()
                    .table_name(c.table.clone())
                    .item(KEY_ATTRIBUTE, AttributeValue::S(c.build_key(&key)))
                    .item(
                        VALUE_ATTRIBUTE,
                        AttributeValue::B(Blob::new(element.as_ref())),
                    )
                    .return_values(ReturnValue::AllOld)
                    .send()
                    .await?
                    .attributes
                    .map(|a| match a.get(VALUE_ATTRIBUTE) {
                        Some(AttributeValue::B(blob)) => Ok(blob.clone().into_inner()),
                        Some(_) => Err(anyhow!("Wrong type for the value")),
                        None => Err(anyhow!("No value field")),
                    })
                    .transpose();
                c.client
                    .update_item()
                    .table_name(c.table.clone())
                    .key(
                        KEY_ATTRIBUTE,
                        AttributeValue::S(format!(
                            "{}/{}/{}/{}",
                            c.orbit, c.subsystem, c.prefix, "elements"
                        )),
                    )
                    .update_expression(format!("ADD {} :elements", ELEMENTS_ATTRIBUTE,))
                    .expression_attribute_values(
                        ":elements",
                        AttributeValue::Ss(vec![hex::encode(key)]),
                    )
                    .send()
                    .await?;
                old
            }
        }
    }

    pub async fn insert_batch(&self, batch: Vec<(Vec<u8>, Vec<u8>)>) -> Result<()> {
        match self {
            KV::Sled(c) => {
                let mut sled_batch = Batch::default();
                for (op, height) in batch.into_iter() {
                    sled_batch.insert(op, height);
                }
                c.apply_batch(sled_batch)?;
            }
            KV::DynamoDB(_) => {
                for (op, height) in batch.into_iter() {
                    // dynamodb has a batch_write_item, but it's "essentially
                    // wrappers around multiple read or write requests" and it
                    // is limited to 25 items
                    self.insert(op, &height).await?;
                }
            }
        }
        Ok(())
    }

    pub async fn remove_batch(&self, batch: Vec<Vec<u8>>) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        match self {
            KV::Sled(c) => {
                let mut sled_batch = Batch::default();
                for op in batch.into_iter() {
                    sled_batch.remove(op);
                }
                c.apply_batch(sled_batch)?;
            }
            KV::DynamoDB(c) => {
                for op in batch.clone().into_iter() {
                    c.client
                        .delete_item()
                        .table_name(c.table.clone())
                        .key(KEY_ATTRIBUTE, AttributeValue::S(c.build_key(op)))
                        .send()
                        .await?;
                }
                c.client
                    .update_item()
                    .table_name(c.table.clone())
                    .key(
                        KEY_ATTRIBUTE,
                        AttributeValue::S(format!(
                            "{}/{}/{}/{}",
                            c.orbit, c.subsystem, c.prefix, "elements"
                        )),
                    )
                    .update_expression(format!("DELETE {} :elements", ELEMENTS_ATTRIBUTE,))
                    .expression_attribute_values(
                        ":elements",
                        AttributeValue::Ss(batch.into_iter().map(hex::encode).collect()),
                    )
                    .send()
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn contains_key<N: AsRef<[u8]>>(&self, key: N) -> Result<bool> {
        match self {
            KV::Sled(c) => Ok(c.contains_key(key)?),
            KV::DynamoDB(c) => {
                match c
                    .client
                    .get_item()
                    .table_name(c.table.clone())
                    .key(KEY_ATTRIBUTE, AttributeValue::S(c.build_key(key)))
                    .projection_expression(KEY_ATTRIBUTE)
                    .send()
                    .await
                {
                    Ok(GetItemOutput { item: Some(_), .. }) => Ok(true),
                    Ok(_) => Ok(false),
                    Err(e) => Err(anyhow!("Error checking on item: {}", e)),
                }
            }
        }
    }

    pub async fn elements(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        match self {
            KV::Sled(c) => Ok(c
                .iter()
                .map(|e| e.map(|ee| (ee.0.to_vec(), ee.1.to_vec())))
                .collect::<Result<Vec<(Vec<u8>, Vec<u8>)>, sled::Error>>()?),
            KV::DynamoDB(c) => {
                let elements = match c
                    .client
                    .get_item()
                    .table_name(c.table.clone())
                    .key(
                        KEY_ATTRIBUTE,
                        AttributeValue::S(format!(
                            "{}/{}/{}/{}",
                            c.orbit, c.subsystem, c.prefix, "elements"
                        )),
                    )
                    .send()
                    .await
                {
                    Ok(GetItemOutput {
                        item: Some(output), ..
                    }) => match output.get(ELEMENTS_ATTRIBUTE) {
                        Some(AttributeValue::Ss(elements)) => elements.clone(),
                        Some(_) => return Err(anyhow!("Wrong type for elements set")),
                        None => return Err(anyhow!("No `{}` column", ELEMENTS_ATTRIBUTE)),
                    },
                    Ok(_) => return Ok(vec![]),
                    Err(e) => return Err(anyhow!("Error checking on item: {}", e)),
                };
                stream::iter(elements.into_iter().map(Ok).collect::<Vec<Result<_>>>())
                    .and_then(|element| async move {
                        let e = hex::decode(element)?;
                        Ok::<(Vec<u8>, Vec<u8>), anyhow::Error>((
                            e.clone(),
                            self.get(e)
                                .await?
                                .ok_or_else(|| anyhow!("Failed to find listed element"))?,
                        ))
                    })
                    .try_collect()
                    .await

                // // TODO handle pagination with into_paginator
                // match c
                //     .client
                //     .query()
                //     .table_name(c.table.clone())
                //     .key_condition_expression(format!("begins_with({}, :prefix)", KEY_ATTRIBUTE))
                //     .expression_attribute_values(
                //         ":prefix",
                //         AttributeValue::S(format!("{}/{}/{}", c.orbit, c.subsystem, c.prefix)),
                //     )
                //     .send()
                //     .await
                // {
                //     Ok(QueryOutput {
                //         items: Some(items), ..
                //     }) => items
                //         .into_iter()
                //         .map(|map| {
                //             let key = match map.get(KEY_ATTRIBUTE) {
                //                 Some(AttributeValue::S(k)) => {
                //                     hex::decode(k.split('/').last().ok_or_else(|| {
                //                         anyhow!("Couldn't split on `/` for: {}", k)
                //                     })?)
                //                     .context("Couldn't decode key")?
                //                 }
                //                 _ => {
                //                     return Err(anyhow!(
                //                         "Row with no string `{}` key",
                //                         KEY_ATTRIBUTE
                //                     ))
                //                 }
                //             };
                //             let value = match map.get(VALUE_ATTRIBUTE) {
                //                 Some(AttributeValue::B(blob)) => blob.clone().into_inner(),
                //                 _ => return Err(anyhow!("Row for `{:?}` with no blob value", key)),
                //             };
                //             Ok((key, value))
                //         })
                //         .collect(),
                //     Ok(QueryOutput { items: None, .. }) => Ok(vec![]),
                //     Err(e) => Err(anyhow!("Error checking on item: {}", e)),
                // }
            }
        }
    }
}
