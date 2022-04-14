use anyhow::Result;
use ipfs::{PeerId, Protocol};
use libipld::Cid;
use libp2p::identity::ed25519::Keypair as Ed25519Keypair;
use rocket::{
    data::{Data, ToByteUnit},
    http::{Header, Status},
    request::{FromRequest, Outcome, Request},
    response::{Responder, Response},
    serde::json::Json,
    State,
};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::RwLock,
};

use crate::{
    auth::{DelegateAuthWrapper, InvokeAuthWrapper, S3Action},
    relay::RelayNode,
    resource::OrbitId,
    s3::{ObjectBuilder, ObjectReader},
};

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

pub struct S3Response(ObjectReader, pub Metadata);

impl S3Response {
    pub fn new(md: Metadata, reader: ObjectReader) -> Self {
        Self(reader, md)
    }
}

impl<'r> Responder<'r, 'static> for S3Response {
    fn respond_to(self, r: &'r Request<'_>) -> rocket::response::Result<'static> {
        Ok(Response::build_from(self.1.respond_to(r)?)
            // must ensure that Metadata::respond_to does not set the body of the response
            .streamed_body(self.0)
            .finalize())
    }
}

#[options("/<_s..>")]
pub async fn cors(_s: PathBuf) {}

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
    let id = ipfs::PublicKey::Ed25519(keypair.public()).to_peer_id();
    s.write()
        .map_err(|_| (Status::InternalServerError, "cant read keys"))?
        .insert(id, keypair);
    Ok(id.to_base58())
}

#[post("/delegate")]
pub fn delegate(_d: DelegateAuthWrapper) -> Result<(), (Status, &'static str)> {
    Ok(())
}

#[post("/invoke", data = "<data>")]
pub async fn invoke(
    i: InvokeAuthWrapper,

    data: Data<'_>,
) -> Result<InvocationResponse, (Status, String)> {
    use InvokeAuthWrapper::*;
    match i {
        Create(orbit_id) => Ok(InvocationResponse::OrbitId(orbit_id)),
        S3(action) => handle_s3_action(action, data).await,
    }
}

pub async fn handle_s3_action(
    action: Box<S3Action>,
    data: Data<'_>,
) -> Result<InvocationResponse, (Status, String)> {
    match *action {
        S3Action::Delete {
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
                        format!("Failed to delete content: {}", e),
                    )
                })?;
            Ok(InvocationResponse::Empty)
        }
        S3Action::Get { orbit, key } => match orbit.service.read(key).await {
            Ok(Some((md, r))) => Ok(InvocationResponse::S3Response(S3Response::new(
                Metadata(md),
                r,
            ))),
            _ => Ok(InvocationResponse::Empty),
        },
        S3Action::List { orbit } => {
            Ok(InvocationResponse::List(
                orbit
                    .service
                    .list()
                    .await
                    .filter_map(|r| {
                        // filter out any non-utf8 keys
                        r.map(|v| std::str::from_utf8(v.as_ref()).ok().map(|s| s.to_string()))
                            .transpose()
                    })
                    .collect::<Result<Vec<String>>>()
                    .map_err(|e| (Status::InternalServerError, e.to_string()))?,
            ))
        }
        S3Action::Metadata { orbit, key } => match orbit.service.get(key).await {
            Ok(Some(content)) => Ok(InvocationResponse::Metadata(Metadata(content.metadata))),
            Err(e) => Err((Status::InternalServerError, e.to_string())),
            Ok(None) => Ok(InvocationResponse::Empty),
        },
        S3Action::Put {
            orbit,
            key,
            metadata,
            auth_ref,
        } => {
            let rm: [([u8; 0], _, _); 0] = [];

            orbit
                .service
                .write(
                    [(
                        ObjectBuilder::new(key.as_bytes().to_vec(), metadata.0, auth_ref),
                        data.open(1u8.gigabytes()),
                    )],
                    rm,
                )
                .await
                .map_err(|e| (Status::InternalServerError, e.to_string()))?;
            Ok(InvocationResponse::Empty)
        }
    }
}

pub enum InvocationResponse {
    OrbitId(OrbitId),
    Empty,
    S3Response(S3Response),
    List(Vec<String>),
    Metadata(Metadata),
}

impl<'r> Responder<'r, 'static> for InvocationResponse {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        match self {
            InvocationResponse::OrbitId(orbit_id) => orbit_id.to_string().respond_to(request),
            InvocationResponse::Empty => ().respond_to(request),
            InvocationResponse::S3Response(response) => response.respond_to(request),
            InvocationResponse::List(keys) => Json(keys).respond_to(request),
            InvocationResponse::Metadata(metadata) => metadata.respond_to(request),
        }
    }
}
