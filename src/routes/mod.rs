use anyhow::Result;
use rocket::{data::ToByteUnit, http::Status, State};
use std::collections::HashMap;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{info_span, Instrument};

use crate::{
    auth_guards::{DataIn, DataOut, InvOut, ObjectHeaders},
    authorization::AuthHeaderGetter,
    config::Config,
    tracing::TracingSpan,
    BlockStage, BlockStores, Kepler,
};
use kepler_core::{
    sea_orm::DbErr,
    storage::{ImmutableReadStore, ImmutableStaging},
    TxError, TxStoreError,
};
use kepler_lib::{
    authorization::{Delegation, Invocation, Resources},
    resource::ResourceId,
    ssi::ucan::capabilities::ability::Ability,
};
pub mod util;
use util::LimitedReader;

#[allow(clippy::let_unit_value)]
pub mod util_routes {
    use super::*;

    #[options("/<_s..>")]
    pub async fn cors(_s: std::path::PathBuf) {}

    #[get("/healthz")]
    pub async fn healthcheck(s: &State<Kepler>) -> Status {
        if s.check_db_connection().await.is_ok() {
            Status::Ok
        } else {
            Status::InternalServerError
        }
    }
}

#[get("/peer/generate/<orbit>")]
pub async fn open_host_key(
    s: &State<Kepler>,
    orbit: &str,
) -> Result<String, (Status, &'static str)> {
    s.stage_key(
        &orbit
            .parse()
            .map_err(|_| (Status::BadRequest, "Invalid orbit ID"))?,
    )
    .await
    .map_err(|_| {
        (
            Status::InternalServerError,
            "Failed to stage keypair for orbit",
        )
    })
}

#[post("/delegate")]
pub async fn delegate(
    d: AuthHeaderGetter<Delegation>,
    req_span: TracingSpan,
    kepler: &State<Kepler>,
) -> Result<String, (Status, String)> {
    let action_label = "delegation";
    let span = info_span!(parent: &req_span.0, "delegate", action = %action_label);
    // Instrumenting async block to handle yielding properly
    async move {
        let timer = crate::prometheus::AUTHORIZED_INVOKE_HISTOGRAM
            .with_label_values(&["delegate"])
            .start_timer();
        let res = kepler
            .delegate(d.0)
            .await
            .map_err(|e| {
                (
                    match e {
                        TxError::OrbitNotFound => Status::NotFound,
                        TxError::Db(DbErr::ConnectionAcquire(_)) => Status::InternalServerError,
                        _ => Status::Unauthorized,
                    },
                    e.to_string(),
                )
            })
            .and_then(|c| {
                c.into_iter()
                    .next()
                    .and_then(|(_, c)| c.committed_events.into_iter().next())
                    .ok_or_else(|| (Status::Unauthorized, "Delegation not committed".to_string()))
            })
            .map(|h| h.to_cid(0x55).to_string());
        timer.observe_duration();
        res
    }
    .instrument(span)
    .await
}

#[post("/invoke", data = "<data>")]
pub async fn invoke(
    i: AuthHeaderGetter<Invocation>,
    req_span: TracingSpan,
    headers: ObjectHeaders,
    data: DataIn<'_>,
    staging: &State<BlockStage>,
    kepler: &State<Kepler>,
    config: &State<Config>,
) -> Result<DataOut<<BlockStores as ImmutableReadStore>::Readable>, (Status, String)> {
    let action_label = "invocation";
    let span = info_span!(parent: &req_span.0, "invoke", action = %action_label);
    // Instrumenting async block to handle yielding properly
    async move {
        let timer = crate::prometheus::AUTHORIZED_INVOKE_HISTOGRAM
            .with_label_values(&["invoke"])
            .start_timer();

        let d = Ability::new("kv/put").unwrap();
        let mut put_iter = Resources::<'_, ResourceId, _>::grants(&i.0 .0).filter_map(|(r, a)| {
            let (o, s, p, _) = r.into_inner();
            match (s.as_deref(), p, a.contains_key(&d)) {
                (Some("kv"), Some(p), true) => Some((o, p)),
                _ => None,
            }
        });

        let inputs = match (data, put_iter.next(), put_iter.next()) {
            (DataIn::None | DataIn::One(_), None, _) => HashMap::new(),
            (DataIn::One(d), Some((orbit, path)), None) => {
                let mut stage = staging
                    .stage(&orbit)
                    .await
                    .map_err(|e| (Status::InternalServerError, e.to_string()))?;
                let open_data = d.open(1u8.gigabytes()).compat();

                if let Some(limit) = config.storage.limit {
                    let current_size = kepler
                        .store_size(&orbit)
                        .await
                        .map_err(|e| (Status::InternalServerError, e.to_string()))?
                        .ok_or_else(|| (Status::NotFound, "orbit not found".to_string()))?;
                    // get the remaining allocated space for the given orbit storage
                    match limit.as_u64().checked_sub(current_size) {
                        // the current size is already equal or greater than the limit
                        None | Some(0) => {
                            return Err((
                                Status::PayloadTooLarge,
                                "The data storage limit has been reached".into(),
                            ))
                        }
                        Some(remaining) => {
                            futures::io::copy(LimitedReader::new(open_data, remaining), &mut stage)
                                .await
                                .map_err(|e| (Status::InternalServerError, e.to_string()))?;
                        }
                    }
                } else {
                    // no limit on storage, just use the data as is
                    futures::io::copy(open_data, &mut stage)
                        .await
                        .map_err(|e| (Status::InternalServerError, e.to_string()))?;
                };

                let mut inputs = HashMap::new();
                inputs.insert((orbit.clone(), path.to_string()), (headers.0, stage));
                inputs
            }
            (DataIn::Many(_), Some(_), Some(_)) => {
                return Err((
                    Status::BadRequest,
                    "Multipart not yet supported".to_string(),
                ));
            }
            _ => {
                return Err((Status::BadRequest, "Invalid inputs".to_string()));
            }
        };
        let res = kepler
            .invoke::<BlockStage>(i.0, inputs)
            .await
            .map(
                |(_, mut outcomes)| match (outcomes.pop(), outcomes.pop(), outcomes.drain(..)) {
                    (None, None, _) => DataOut::None,
                    (Some(o), None, _) => DataOut::One(InvOut(o)),
                    (Some(o), Some(next), rest) => {
                        let mut v = vec![InvOut(o), InvOut(next)];
                        v.extend(rest.map(InvOut));
                        DataOut::Many(v)
                    }
                    _ => unreachable!(),
                },
            )
            .map_err(|e| {
                (
                    match e {
                        TxStoreError::Tx(TxError::OrbitNotFound) => Status::NotFound,
                        TxStoreError::Tx(TxError::Db(DbErr::ConnectionAcquire(_))) => {
                            Status::InternalServerError
                        }
                        _ => Status::Unauthorized,
                    },
                    e.to_string(),
                )
            });

        timer.observe_duration();
        res
    }
    .instrument(span)
    .await
}
