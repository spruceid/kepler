use anyhow::Result;
use libp2p::{
    identity::{Keypair, PeerId},
    multiaddr::Protocol,
};
use rocket::{data::ToByteUnit, http::Status, State};
use std::{collections::HashMap, sync::RwLock};
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{info_span, Instrument};

use crate::{
    auth_guards::{DataIn, DataOut, InvOut, ObjectHeaders},
    authorization::AuthHeaderGetter,
    relay::RelayNode,
    tracing::TracingSpan,
    BlockStage, BlockStores, Kepler,
};
use kepler_core::{
    storage::{ImmutableReadStore, ImmutableStaging},
    types::Resource,
    util::{DelegationInfo, InvocationInfo},
    TxError,
};

#[allow(clippy::let_unit_value)]
pub mod util_routes {
    #[options("/<_s..>")]
    pub async fn cors(_s: std::path::PathBuf) {}

    #[get("/healthz")]
    pub fn healthcheck() {}
}

#[get("/peer/relay")]
pub fn relay_addr(relay: &State<RelayNode>) -> String {
    relay
        .external()
        .with(Protocol::P2p(relay.id.into()))
        .to_string()
}

#[get("/peer/generate")]
pub fn open_host_key(
    s: &State<RwLock<HashMap<PeerId, Keypair>>>,
) -> Result<String, (Status, &'static str)> {
    let keypair = Keypair::generate_ed25519();
    let id = keypair.public().to_peer_id();
    s.write()
        .map_err(|_| (Status::InternalServerError, "cant read keys"))?
        .insert(id, keypair);
    Ok(id.to_base58())
}

#[post("/delegate")]
pub async fn delegate(
    d: AuthHeaderGetter<DelegationInfo>,
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
    i: AuthHeaderGetter<InvocationInfo>,
    req_span: TracingSpan,
    headers: ObjectHeaders,
    data: DataIn<'_>,
    staging: &State<BlockStage>,
    kepler: &State<Kepler>,
) -> Result<DataOut<<BlockStores as ImmutableReadStore>::Readable>, (Status, String)> {
    let action_label = "invocation";
    let span = info_span!(parent: &req_span.0, "invoke", action = %action_label);
    // Instrumenting async block to handle yielding properly
    async move {
        let timer = crate::prometheus::AUTHORIZED_INVOKE_HISTOGRAM
            .with_label_values(&["invoke"])
            .start_timer();

        let mut put_iter =
            i.0 .0
                .capabilities
                .iter()
                .filter_map(|c| match (&c.resource, c.action.as_str()) {
                    (Resource::Kepler(r), "put") if r.service() == Some("kv") => {
                        r.path().map(|p| (r.orbit(), p))
                    }
                    _ => None,
                });

        let inputs = match (data, put_iter.next(), put_iter.next()) {
            (DataIn::None | DataIn::One(_), None, _) => HashMap::new(),
            (DataIn::One(d), Some((orbit, path)), None) => {
                let mut stage = staging
                    .stage(orbit)
                    .await
                    .map_err(|e| (Status::InternalServerError, e.to_string()))?;
                futures::io::copy(d.open(1u32.gibibytes()).compat(), &mut stage)
                    .await
                    .map_err(|e| (Status::InternalServerError, e.to_string()))?;
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
            .map_err(|e| (Status::Unauthorized, e.to_string()));

        timer.observe_duration();
        res
    }
    .instrument(span)
    .await
}
