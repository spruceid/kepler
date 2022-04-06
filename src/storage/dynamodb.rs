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
use ipfs::{
    refs::IpldRefsError,
    repo::{PinKind, PinMode, PinStore},
};
use libipld::cid::{multibase::Base, Cid};
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

// TODO implement mutex

// TODO make that public in rust-ipfs
pub type References<'a> = futures::stream::BoxStream<'a, Result<Cid, IpldRefsError>>;

#[async_trait]
impl PinStore for DynamoPinStore {
    async fn is_pinned(&self, cid: &Cid) -> Result<bool, Error> {
        match self
            .client
            .lock()
            .await
            .get_item()
            .table_name(self.table.clone())
            .key(
                CID_ATTRIBUTE,
                AttributeValue::S(format!(
                    "{}/{}",
                    self.orbit.to_string_of_base(Base::Base58Btc)?,
                    cid
                )),
            )
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(SdkError::ServiceError {
                err:
                    GetItemError {
                        kind: GetItemErrorKind::ResourceNotFoundException(_),
                        ..
                    },
                ..
            }) => Ok(false),
            Err(e) => Err(anyhow!("Error checking on item: {}", e)),
        }
    }

    /// Only using recursive pins
    async fn insert_direct_pin(&self, _target: &Cid) -> Result<(), Error> {
        Ok(())
        // Assuming a direct pin can't be already recursively/indirectly pinned already (i.e. already exist);
    }

    async fn insert_recursive_pin(
        &self,
        target: &Cid,
        referenced: References<'_>,
    ) -> Result<(), Error> {
        let client = self.client.lock().await;
        // TODO either insert or increment
        client
            .update_item()
            .table_name(self.table.clone())
            .key(
                CID_ATTRIBUTE,
                AttributeValue::S(format!(
                    "{}/{}",
                    self.orbit.to_string_of_base(Base::Base58Btc)?,
                    target
                )),
            )
            .update_expression(format!(
                "SET {} = :pin, {p} = {p} + :increment",
                ROOT_ATTRIBUTE,
                p = PARENTS_ATTRIBUTE
            ))
            .expression_attribute_values(":pin", AttributeValue::Bool(true))
            .expression_attribute_values(":increment", AttributeValue::N(1.to_string()))
            .send()
            .await?;

        let set = referenced.try_collect::<BTreeSet<_>>().await?;
        for cid in set.iter() {
            client
                .update_item()
                .table_name(self.table.clone())
                .key(
                    CID_ATTRIBUTE,
                    AttributeValue::S(format!(
                        "{}/{}",
                        self.orbit.to_string_of_base(Base::Base58Btc)?,
                        cid
                    )),
                )
                .update_expression(format!("SET {p} = {p} + :increment", p = PARENTS_ATTRIBUTE))
                .expression_attribute_values(":increment", AttributeValue::N(1.to_string()))
                .send()
                .await?;
        }
        Ok(())
    }

    /// Only using recursive pins
    async fn remove_direct_pin(&self, _target: &Cid) -> Result<(), Error> {
        Ok(())
    }

    async fn remove_recursive_pin(
        &self,
        target: &Cid,
        referenced: References<'_>,
    ) -> Result<(), Error> {
        let client = self.client.lock().await;
        let res = client
            .update_item()
            .table_name(self.table.clone())
            .key(
                CID_ATTRIBUTE,
                AttributeValue::S(format!(
                    "{}/{}",
                    self.orbit.to_string_of_base(Base::Base58Btc)?,
                    target
                )),
            )
            .update_expression(format!(
                "SET {} = :pin, {p} = {p} - :increment",
                ROOT_ATTRIBUTE,
                p = PARENTS_ATTRIBUTE
            ))
            .expression_attribute_values(":pin", AttributeValue::Bool(false))
            .expression_attribute_values(":increment", AttributeValue::N(1.to_string()))
            .return_values(ReturnValue::UpdatedNew)
            .send()
            .await?;

        // TODO use a conditional delete
        match res
            .attributes
            .and_then(|m| m.get(PARENTS_ATTRIBUTE).cloned())
        {
            Some(AttributeValue::N(parents)) => {
                if *parents == 0.to_string() {
                    client
                        .delete_item()
                        .table_name(self.table.clone())
                        .key(
                            CID_ATTRIBUTE,
                            AttributeValue::S(format!(
                                "{}/{}",
                                self.orbit.to_string_of_base(Base::Base58Btc)?,
                                target
                            )),
                        )
                        .send()
                        .await?;
                }
            }
            _ => error!("No attribute returned."),
        };

        let set = referenced.try_collect::<BTreeSet<_>>().await?;
        for cid in set.iter() {
            let res = client
                .update_item()
                .table_name(self.table.clone())
                .key(
                    CID_ATTRIBUTE,
                    AttributeValue::S(format!(
                        "{}/{}",
                        self.orbit.to_string_of_base(Base::Base58Btc)?,
                        cid
                    )),
                )
                .update_expression(format!("SET {p} = {p} - :increment", p = PARENTS_ATTRIBUTE))
                .expression_attribute_values(":increment", AttributeValue::N(1.to_string()))
                .return_values(ReturnValue::UpdatedNew)
                .send()
                .await?;

            match res.attributes.map(|m| m.get(PARENTS_ATTRIBUTE).cloned()) {
                Some(Some(AttributeValue::N(parents))) => {
                    if *parents == 0.to_string() {
                        client
                            .delete_item()
                            .table_name(self.table.clone())
                            .key(
                                CID_ATTRIBUTE,
                                AttributeValue::S(format!(
                                    "{}/{}",
                                    self.orbit.to_string_of_base(Base::Base58Btc)?,
                                    target
                                )),
                            )
                            .send()
                            .await?;
                    }
                }
                _ => error!("No attribute returned."),
            };
        }
        Ok(())
    }

    async fn list(
        &self,
        requirement: Option<PinMode>,
    ) -> futures::stream::BoxStream<'static, Result<(Cid, PinMode), Error>> {
        let query = self
            .client
            .lock()
            .await
            .scan()
            .table_name(self.table.clone());
        let query = match requirement {
            Some(PinMode::Recursive | PinMode::Direct) => query
                .filter_expression(format!("{} = :r", ROOT_ATTRIBUTE))
                .expression_attribute_values(":r", AttributeValue::Bool(true)),
            None | Some(PinMode::Indirect) => query,
        };
        // TODO handle pagination
        match query.send().await {
            Ok(ScanOutput {
                items: Some(items), ..
            }) => stream::iter(items)
                .map(|map| {
                    let cid = match map.get(CID_ATTRIBUTE) {
                        Some(AttributeValue::S(c)) => c,
                        _ => return Err(anyhow!("Row with no string Cid key.")),
                    };
                    let pin_mode = match map.get(ROOT_ATTRIBUTE) {
                        Some(AttributeValue::Bool(true)) => PinMode::Recursive,
                        Some(AttributeValue::Bool(false)) => PinMode::Indirect,
                        Some(_) | None => {
                            error!("Cid `{}` with no boolean Root attribute.", cid);
                            return Err(anyhow!("Cid `{}` with no boolean Root attribute.", cid));
                        }
                    };
                    Cid::from_str(cid)
                        .map_err(|e| anyhow!("Couldn't convert cid key to Cid: {}", e))
                        .map(|kk| (kk, pin_mode))
                })
                .boxed(),
            Ok(ScanOutput { items: None, .. }) => stream::iter(vec![]).boxed(),
            Err(e) => stream::iter(vec![Err(anyhow!("Error checking on item: {}", e))]).boxed(),
        }
    }

    async fn query(
        &self,
        ids: Vec<Cid>,
        requirement: Option<PinMode>,
    ) -> Result<Vec<(Cid, PinKind<Cid>)>, Error> {
        // TODO impl PinKind::IndirectFrom
        let query = self
            .client
            .lock()
            .await
            .scan()
            .table_name(self.table.clone());
        let (query, expression) = match requirement {
            Some(PinMode::Recursive | PinMode::Direct) => (
                query.expression_attribute_values(":r", AttributeValue::Bool(true)),
                format!("{} == :r and", ROOT_ATTRIBUTE),
            ),
            None | Some(PinMode::Indirect) => (query, String::new()),
        };
        let query = query
            .filter_expression(format!("{} contains (:ids, {})", expression, CID_ATTRIBUTE))
            .expression_attribute_values(
                ":ids",
                AttributeValue::Ns(
                    ids.iter()
                        .map(|id| {
                            self.orbit
                                .to_string_of_base(Base::Base58Btc)
                                .map(|c| format!("{}/{}", c, id))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            );
        // TODO handle pagination
        match query.send().await {
            Ok(ScanOutput {
                items: Some(items), ..
            }) => items
                .iter()
                .map(|map| {
                    let cid = match map.get(CID_ATTRIBUTE) {
                        Some(AttributeValue::S(c)) => c,
                        _ => return Err(anyhow!("Row with no string Cid key.")),
                    };
                    let pin_mode = match map.get(ROOT_ATTRIBUTE) {
                        Some(AttributeValue::Bool(true)) => PinKind::Recursive(0),
                        Some(AttributeValue::Bool(false)) => PinKind::IndirectFrom(Cid::default()),
                        Some(_) | None => {
                            error!("Cid `{}` with no boolean Root attribute.", cid);
                            return Err(anyhow!("Cid `{}` with no boolean Root attribute.", cid));
                        }
                    };
                    Cid::from_str(cid)
                        .map_err(|e| anyhow!("Couldn't convert cid key to Cid: {}", e))
                        .map(|kk| (kk, pin_mode))
                })
                .collect(),
            Ok(ScanOutput { items: None, .. }) => Ok(vec![]),
            Err(e) => Err(anyhow!("Error checking on item: {}", e)),
        }
    }
}
