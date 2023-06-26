use anyhow::Result;
use kepler_core::{
    types::Metadata,
    util::{Capability, DelegationInfo},
    InvocationOutcome,
};
use kepler_lib::{
    authorization::{EncodingError, HeaderEncode},
    libipld::cid::Cid,
    resource::{OrbitId, ResourceId},
};
use rocket::{
    data::{Capped, FromData},
    futures::io::AsyncRead,
    http::{ContentType, Header, Status},
    outcome::Outcome as DataOutcome,
    request::{FromRequest, Outcome, Request},
    response::{Responder, Response},
    serde::json::Json,
    Data,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use thiserror::Error;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tracing::{info_span, Instrument};

pub fn simple_check(target: &ResourceId, capability: &ResourceId) -> Result<()> {
    check_orbit_and_service(target, capability)?;
    simple_prefix_check(target, capability)?;
    simple_check_fragments(target, capability)
}

pub fn simple_check_fragments(target: &ResourceId, capability: &ResourceId) -> Result<()> {
    match (target.fragment(), capability.fragment()) {
        (Some(t), Some(c)) if t == c => Ok(()),
        _ => Err(anyhow!("Target Action does not match Capability")),
    }
}

pub fn simple_prefix_check(target: &ResourceId, capability: &ResourceId) -> Result<()> {
    // if #action is same
    // Ok if target.path => cap.path
    if target.service() == capability.service()
        && match (target.path(), capability.path()) {
            (Some(t), Some(c)) => t.starts_with(c),
            (_, None) => true,
            _ => false,
        }
    {
        Ok(())
    } else {
        Err(anyhow!("Target Service and Path are not correct"))
    }
}

#[derive(Error, Debug)]
pub enum TargetCheckError {
    #[error("Invocation and Capability Orbits do not match")]
    IncorrectOrbit,
    #[error("Invocation and Capability Services do not match")]
    IncorrectService,
}

pub fn check_orbit_and_service(
    target: &ResourceId,
    capability: &ResourceId,
) -> Result<(), TargetCheckError> {
    if target.orbit() != capability.orbit() {
        Err(TargetCheckError::IncorrectOrbit)
    } else if target.service() != capability.service() {
        Err(TargetCheckError::IncorrectService)
    } else {
        Ok(())
    }
}

#[derive(Debug)]
pub enum DataHolder<O, M = O> {
    None,
    One(O),
    Many(Vec<M>),
}

#[derive(Debug)]
pub struct InvOut<R>(pub InvocationOutcome<R>);

pub type DataIn<'a> = DataHolder<Data<'a>, (OrbitId, String, Metadata, Capped<&'a [u8]>)>;
pub type DataOut<R> = DataHolder<InvOut<R>>;

#[async_trait]
impl<'r> FromData<'r> for DataIn<'r> {
    type Error = anyhow::Error;

    async fn from_data(
        req: &'r Request<'_>,
        data: Data<'r>,
    ) -> DataOutcome<Self, (Status, Self::Error), Data<'r>> {
        let req_span = req
            .local_cache(|| Option::<crate::tracing::TracingSpan>::None)
            .as_ref()
            .unwrap();
        let span = info_span!(parent: &req_span.0, "data_in");
        // Instrumenting async block to handle yielding properly
        async move {
            let timer = crate::prometheus::AUTHORIZATION_HISTOGRAM
                .with_label_values(&["invoke"])
                .start_timer();

            let res = match <&'r ContentType>::from_request(req).await.succeeded() {
                Some(c) if c.is_form_data() => DataOutcome::Failure((
                    Status::BadRequest,
                    anyhow!("Multipart uploads not yet supported"),
                )),
                Some(_) => DataOutcome::Success(DataIn::One(data)),
                None => DataOutcome::Forward(data),
            };

            timer.observe_duration();
            res
        }
        .instrument(span)
        .await
    }
}

impl<'r, R> Responder<'r, 'static> for InvOut<R>
where
    R: 'static + AsyncRead + Send,
{
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        match self.0 {
            InvocationOutcome::KvList(list) => Json(list).respond_to(request),
            InvocationOutcome::KvDelete => ().respond_to(request),
            InvocationOutcome::KvMetadata(meta) => meta.map(ObjectHeaders).respond_to(request),
            InvocationOutcome::KvWrite => ().respond_to(request),
            InvocationOutcome::KvRead(data) => {
                data.map(|(md, c)| KVResponse(c, md)).respond_to(request)
            }
            InvocationOutcome::OpenSessions(sessions) => Json(
                sessions
                    .into_iter()
                    .map(|(hash, del)| {
                        Ok((
                            hash.to_cid(0x55).to_string(),
                            CapJsonRep::from_delegation(del)?,
                        ))
                    })
                    .collect::<Result<HashMap<String, CapJsonRep>>>()
                    .map_err(|_| Status::InternalServerError)?,
            )
            .respond_to(request),
        }
    }
}

impl<'r, R> Responder<'r, 'static> for DataOut<R>
where
    R: 'static + AsyncRead + Send,
{
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        match self {
            DataHolder::None => ().respond_to(request),
            DataHolder::One(inv) => inv.respond_to(request),
            DataHolder::Many(_invs) => Err(Status::NotImplemented),
        }
    }
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
    pub fn from_delegation(d: DelegationInfo) -> Result<Self, EncodingError> {
        Ok(Self {
            capabilities: d.capabilities,
            delegator: d.delegator,
            delegate: d.delegate,
            parents: d.parents,
            raw: d.delegation.encode()?,
        })
    }
}

pub struct ObjectHeaders(pub Metadata);

#[async_trait]
impl<'r> FromRequest<'r> for ObjectHeaders {
    type Error = anyhow::Error;
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let md: BTreeMap<String, String> = request
            .headers()
            .iter()
            .map(|h| (h.name.into_string(), h.value.to_string()))
            .collect();
        Outcome::Success(ObjectHeaders(Metadata(md)))
    }
}

impl<'r> Responder<'r, 'static> for ObjectHeaders {
    fn respond_to(self, _: &'r Request<'_>) -> rocket::response::Result<'static> {
        let mut r = Response::build();
        for (k, v) in self.0 .0 {
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
        Ok(Response::build_from(ObjectHeaders(self.1).respond_to(r)?)
            // must ensure that Metadata::respond_to does not set the body of the response
            .streamed_body(self.0.compat())
            .finalize())
    }
}
