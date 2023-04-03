use anyhow::Result;
use futures::io::AsyncRead;
use kepler_lib::{
    authorization::{EncodingError, HeaderEncode},
    libipld::Cid,
};
use libp2p::{
    core::PeerId,
    identity::{ed25519::Keypair as Ed25519Keypair, PublicKey},
    multiaddr::Protocol,
};
use rocket::{
    data::{Data, ToByteUnit},
    http::{Header, Status},
    request::{FromRequest, Outcome, Request},
    response::{Responder, Response},
    serde::json::Json,
    State,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::RwLock,
};
use tracing::{info_span, Instrument};

use crate::{
    auth_guards::{CapAction, DelegateAuthWrapper, InvokeAuthWrapper, KVAction},
    authorization::{Capability, Delegation},
    kv::{ObjectBuilder, ReadResponse},
    relay::RelayNode,
    storage::{Content, ImmutableStore},
    tracing::TracingSpan,
    BlockStores, Config,
};
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

pub struct Metadata(pub BTreeMap<String, String>);

#[async_trait]
impl<'r> FromRequest<'r> for Metadata {
    type Error = anyhow::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let md: BTreeMap<String, String> = request
            .headers()
            .iter()
            .map(|h| (h.name.into_string(), h.value.to_string()))
            .collect();
        Outcome::Success(Metadata(md))
    }
}

impl<'r> Responder<'r, 'static> for Metadata {
    fn respond_to(self, _: &'r Request<'_>) -> rocket::response::Result<'static> {
        let mut r = Response::build();
        for (k, v) in self.0 {
            if k != "content-length" {
                r.header(Header::new(k, v));
            }
        }
        Ok(r.finalize())
    }
}

pub struct KVResponse<R>(R, pub Metadata);

impl<R> KVResponse<R> {
    pub fn new(md: Metadata, reader: R) -> Self {
        Self(reader, md)
    }
}

impl<'r, R> Responder<'r, 'static> for KVResponse<R>
where
    R: 'static + AsyncRead + Send,
{
    fn respond_to(self, r: &'r Request<'_>) -> rocket::response::Result<'static> {
        Ok(Response::build_from(self.1.respond_to(r)?)
            // must ensure that Metadata::respond_to does not set the body of the response
            .streamed_body(self.0.compat())
            .finalize())
    }
}

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
    s: &State<RwLock<HashMap<PeerId, Ed25519Keypair>>>,
) -> Result<String, (Status, &'static str)> {
    let keypair = Ed25519Keypair::generate();
    let id = PublicKey::Ed25519(keypair.public()).to_peer_id();
    s.write()
        .map_err(|_| (Status::InternalServerError, "cant read keys"))?
        .insert(id, keypair);
    Ok(id.to_base58())
}

#[post("/delegate")]
pub fn delegate(d: DelegateAuthWrapper) -> DelegateAuthWrapper {
    d
}

impl<'r> Responder<'r, 'static> for DelegateAuthWrapper {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        match self {
            DelegateAuthWrapper::OrbitCreation(orbit_id) => {
                orbit_id.to_string().respond_to(request)
            }
            DelegateAuthWrapper::Delegation(c) => c.to_string().respond_to(request),
        }
    }
}

#[post("/invoke", data = "<data>")]
pub async fn invoke(
    i: InvokeAuthWrapper<BlockStores>,
    req_span: TracingSpan,
    data: Data<'_>,
    config: &State<Config>,
) -> Result<InvocationResponse<Content<<BlockStores as ImmutableStore>::Readable>>, (Status, String)>
{
    let action_label = i.prometheus_label().to_string();
    let span = info_span!(parent: &req_span.0, "invoke", action = %action_label);
    // Instrumenting async block to handle yielding properly
    async move {
        let timer = crate::prometheus::AUTHORIZED_INVOKE_HISTOGRAM
            .with_label_values(&[&action_label])
            .start_timer();

        let res = match i {
            InvokeAuthWrapper::Revocation => Ok(InvocationResponse::Revoked),
            InvokeAuthWrapper::KV(action) => handle_kv_action(*action, data, config).await,
            InvokeAuthWrapper::CapabilityQuery(action) => handle_cap_action(*action, data).await,
        };

        timer.observe_duration();
        res
    }
    .instrument(span)
    .await
}

pub async fn handle_kv_action<B>(
    action: KVAction<B>,
    data: Data<'_>,
    config: &Config,
) -> Result<InvocationResponse<Content<B::Readable>>, (Status, String)>
where
    B: 'static + ImmutableStore,
{
    match action {
        KVAction::Delete {
            orbit,
            key,
            auth_ref,
        } => {
            let add: Vec<(&[u8], Cid)> = vec![];
            orbit
                .service
                .index(add, vec![(key, None, auth_ref)])
                .await
                .map_err(|e| {
                    (
                        Status::InternalServerError,
                        format!("Failed to delete content: {e}"),
                    )
                })?;
            Ok(InvocationResponse::EmptySuccess)
        }
        KVAction::Get { orbit, key } => match orbit.service.read(key).await {
            Ok(Some(ReadResponse(md, r))) => Ok(InvocationResponse::KVResponse(KVResponse::new(
                Metadata(md),
                r,
            ))),
            Err(e) => Err((Status::InternalServerError, e.to_string())),
            Ok(None) => Ok(InvocationResponse::NotFound),
        },
        KVAction::List { orbit, prefix } => {
            Ok(InvocationResponse::List(
                orbit
                    .service
                    .list()
                    .await
                    .filter_map(|r| {
                        // filter out non-utf8 keys and those not matching the prefix
                        r.map(|v| {
                            std::str::from_utf8(v.as_ref())
                                .ok()
                                .map(|s| s.to_string())
                                .filter(|key| key.starts_with(&prefix))
                        })
                        .transpose()
                    })
                    .collect::<Result<Vec<String>>>()
                    .map_err(|e| (Status::InternalServerError, e.to_string()))?,
            ))
        }
        KVAction::Metadata { orbit, key } => match orbit.service.get(key).await {
            Ok(Some(content)) => Ok(InvocationResponse::Metadata(Metadata(content.metadata))),
            Err(e) => Err((Status::InternalServerError, e.to_string())),
            Ok(None) => Ok(InvocationResponse::NotFound),
        },
        KVAction::Put {
            orbit,
            key,
            metadata,
            auth_ref,
        } => {
            let rm: [([u8; 0], _, _); 0] = [];

            let size = orbit
                .service
                .store
                .blocks()
                .total_size()
                .await
                .map_err(|e| (Status::InternalServerError, e.to_string()))?;

            let allowed_size = config
                .storage
                .limit
                .map(|l| if l > size { l - size } else { 0.bytes() })
                .unwrap_or(1u8.gigabytes());

            orbit
                .service
                .write(
                    [(
                        ObjectBuilder::new(key.as_bytes().to_vec(), metadata.0, auth_ref),
                        data.open(allowed_size).compat(),
                    )],
                    rm,
                )
                .await
                .map_err(|e| (Status::InternalServerError, e.to_string()))?;
            Ok(InvocationResponse::EmptySuccess)
        }
    }
}

pub async fn handle_cap_action<B>(
    action: CapAction<B>,
    _data: Data<'_>,
) -> Result<InvocationResponse<Content<B::Readable>>, (Status, String)>
where
    B: 'static + ImmutableStore,
{
    match action {
        CapAction::Query {
            orbit,
            query,
            invoker,
        } => orbit
            .capabilities
            .store
            .query(query, &invoker)
            .await
            .map(InvocationResponse::CapabilityQuery)
            .map_err(|e| (Status::InternalServerError, e.to_string())),
    }
}

pub enum InvocationResponse<R> {
    NotFound,
    EmptySuccess,
    KVResponse(KVResponse<R>),
    List(Vec<String>),
    Metadata(Metadata),
    CapabilityQuery(HashMap<Cid, Delegation>),
    Revoked,
}

#[derive(Serialize, Deserialize)]
pub struct CapJsonRep {
    pub capabilities: Vec<Capability>,
    pub delegator: String,
    pub delegate: String,
    pub parents: Vec<Cid>,
    raw: String,
}

impl CapJsonRep {
    pub fn from_delegation(d: Delegation) -> Result<Self, EncodingError> {
        Ok(Self {
            capabilities: d.capabilities,
            delegator: d.delegator,
            delegate: d.delegate,
            parents: d.parents,
            raw: d.delegation.encode()?,
        })
    }
}

impl<'r, R> Responder<'r, 'static> for InvocationResponse<R>
where
    R: 'static + AsyncRead + Send,
{
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        match self {
            InvocationResponse::NotFound => Option::<()>::None.respond_to(request),
            InvocationResponse::EmptySuccess => ().respond_to(request),
            InvocationResponse::KVResponse(response) => response.respond_to(request),
            InvocationResponse::List(keys) => Json(keys).respond_to(request),
            InvocationResponse::Metadata(metadata) => metadata.respond_to(request),
            InvocationResponse::Revoked => ().respond_to(request),
            InvocationResponse::CapabilityQuery(caps) => Json(
                caps.into_iter()
                    .map(|(cid, del)| Ok((cid.to_string(), CapJsonRep::from_delegation(del)?)))
                    .collect::<Result<HashMap<String, CapJsonRep>>>()
                    .map_err(|_| Status::InternalServerError)?,
            )
            .respond_to(request),
        }
    }
}
