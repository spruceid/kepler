use axum::{
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use kepler_lib::{
    didkit::{Source, DID_METHODS},
    resource::{OrbitId, ResourceId},
    ssi::{
        cacao_zcap::CacaoZcapExtraProps,
        jwk::JWK,
        ldp::LinkedDataProofs,
        vc::{get_verification_method, LinkedDataProofOptions, ProofPurpose, URI},
        zcap::{Contexts, Invocation},
    },
    zcap::{InvProps, KeplerDelegation, KeplerInvocation},
};
use serde::{Deserialize, Serialize};
use serde_json::{self, Value};
use std::{net::SocketAddr, sync::Arc};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct InvokeParams {
    name: String,
    action: String,
}

async fn invocation(
    Json(params): Json<InvokeParams>,
    // Path(resource_id): Path<ResourceId>,
    Extension(jwk): Extension<Arc<JWK>>,
) -> Json<KeplerInvocation> {
    let did = DID_METHODS
        .get("key")
        .unwrap()
        .generate(&Source::Key(&jwk))
        .unwrap();
    let orbit_id = OrbitId::new(
        did.strip_prefix("did:").unwrap().to_string(),
        String::from("default"),
    );
    let invocation_target = orbit_id.to_resource(
        Some("kv".to_string()),
        Some(params.name),
        Some(params.action),
    );

    let invocation = {
        let context = Contexts::default();
        let id = URI::String(format!("urn:uuid:{}", Uuid::new_v4()));
        let property_set = InvProps {
            invocation_target,
            expires: None,
            valid_from: None,
            extra_fields: None,
        };
        KeplerInvocation {
            context,
            id,
            property_set,
            proof: None,
        }
    };

    let resolver = DID_METHODS.to_resolver();
    let verification_method = get_verification_method(&did, resolver).await.unwrap();
    let ldp_options = LinkedDataProofOptions {
        verification_method: Some(URI::String(verification_method)),
        proof_purpose: Some(ProofPurpose::CapabilityInvocation),
        ..Default::default()
    };
    // let mut capability_chain = delegation
    //     .proof
    //     .as_ref()
    //     .and_then(|proof| proof.property_set.as_ref())
    //     .and_then(|props| props.get("capabilityChain"))
    //     .and_then(|chain| chain.as_array().cloned())
    //     .unwrap_or_default();
    // capability_chain.push(serde_json::to_value(&delegation).unwrap());

    let mut proof_props = std::collections::HashMap::<String, Value>::new();
    // proof_props.insert(
    //     "capabilityChain".into(),
    //     serde_json::to_value(capability_chain).unwrap(),
    // );

    let proof =
        LinkedDataProofs::sign(&invocation, &ldp_options, resolver, &jwk, Some(proof_props))
            .await
            .unwrap();
    Json(invocation.set_proof(proof))
}

#[derive(Serialize, Deserialize)]
struct OrbitParams {
    peer_id: String,
}

async fn create_orbit(
    Json(params): Json<OrbitParams>,
    Extension(jwk): Extension<Arc<JWK>>,
) -> Json<KeplerDelegation> {
    let did = DID_METHODS
        .get("key")
        .unwrap()
        .generate(&Source::Key(&jwk))
        .unwrap();
    let id = URI::String(format!("urn:uuid:{}", Uuid::new_v4()));
    let orbit_id = OrbitId::new(
        did.strip_prefix("did:").unwrap().to_string(),
        String::from("default"),
    );
    let delegation_extraprops = CacaoZcapExtraProps {
        r#type: "CacaoZcap2022".to_string(),
        expires: None,
        valid_from: None,
        invocation_target: orbit_id.to_string(),
        cacao_payload_type: header_type,
        allowed_action: None,
        cacao_zcap_substatement: None,
        cacao_request_id: None,
    };
    let delegation = KeplerDelegation::new(id, id, delegation_extraprops);

    let resolver = DID_METHODS.to_resolver();
    let verification_method = get_verification_method(&did, resolver).await.unwrap();
    let ldp_options = LinkedDataProofOptions {
        verification_method: Some(URI::String(verification_method)),
        proof_purpose: Some(ProofPurpose::CapabilityDelegation),
        ..Default::default()
    };
    let proof = delegation
        .generate_proof(&jwk, &ldp_options, resolver, &vec![])
        .await
        .unwrap();

    Json(delegation.set_proof(proof))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let app = Router::new()
        .route("/invoke", post(invocation))
        .route("/orbit", post(create_orbit))
        .layer(Extension(Arc::new(JWK::generate_ed25519().unwrap())));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
