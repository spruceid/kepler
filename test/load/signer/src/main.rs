use axum::{routing::post, Extension, Json, Router};
use chrono::prelude::*;
use ethers::{
    core::utils::to_checksum,
    signers::{LocalWallet, Signer},
};
use iri_string::types::UriString;
use kepler_lib::{
    didkit::{Source, DID_METHODS},
    resource::OrbitId,
    ssi::{
        cacao_zcap::{
            cacaos::{
                siwe::{nonce::generate_nonce, Message, Version},
                siwe_cacao::{SIWESignature, SignInWithEthereum},
                BasicSignature, CACAO,
            },
            translation::cacao_to_zcap::cacao_to_zcap,
        },
        jwk::JWK,
        ldp::LinkedDataProofs,
        vc::{get_verification_method, LinkedDataProofOptions, ProofPurpose, URI},
        zcap::Contexts,
    },
    zcap::{InvProps, KeplerDelegation, KeplerInvocation},
};
use serde::{Deserialize, Serialize};
use serde_json::{self, Value};
use std::{net::SocketAddr, sync::Arc};
use uuid::Uuid;

struct User {
    wallet: LocalWallet,
    address: String,
    orbit_id: OrbitId,
    did: String,
    delegation: KeplerDelegation,
}

async fn new_user(wallet: LocalWallet, jwk: JWK) -> User {
    let address = to_checksum(&wallet.address(), None);
    let did = format!("did:pkh:eip155:1:{}", address);
    let orbit_id = OrbitId::new(
        did.strip_prefix("did:").unwrap().to_string(),
        String::from("default"),
    );
    let did = DID_METHODS
        .generate(&Source::KeyAndPattern(&jwk, "key"))
        .unwrap();
    let did_resolver = DID_METHODS.to_resolver();
    let verification_method = get_verification_method(&did, did_resolver).await.unwrap();

    let root_cap = orbit_id.to_string().try_into().unwrap();
    let invocation_target = orbit_id
        .clone()
        .to_resource(Some("kv".to_string()), None, None)
        .to_string()
        .try_into()
        .unwrap();
    let message = Message {
        address: wallet.address().try_into().unwrap(),
        chain_id: wallet.chain_id(),
        domain: "localhost".try_into().unwrap(),
        issued_at: Utc::now().into(),
        nonce: generate_nonce(),
        version: Version::V1,
        not_before: None,
        expiration_time: None,
        request_id: None,
        resources: vec![invocation_target, root_cap],
        statement: Some(format!(
            "Authorize action{}: Allow access to your Kepler orbit using this session key.",
            { format!(" ({})", vec!["put", "get"].join(", ")) }
        )),
        uri: verification_method.try_into().unwrap(),
    };
    let signature = wallet.sign_message(message.to_string()).await.unwrap();
    let delegation = cacao_to_zcap(&CACAO::<SignInWithEthereum>::new(
        message.into(),
        BasicSignature::<SIWESignature> {
            s: signature.to_vec().try_into().unwrap(),
        },
    ))
    .unwrap();
    User {
        wallet,
        address,
        did,
        orbit_id,
        delegation,
    }
}

#[derive(Serialize, Deserialize)]
struct InvokeParams {
    name: String,
    action: String,
}

// /// Attempt at bypassing the session key
// async fn invocation(
//     Json(params): Json<InvokeParams>,
//     Extension(wallet): Extension<Arc<LocalWallet>>,
// ) -> Json<KeplerInvocation> {
//     let address = to_checksum(&wallet.address(), None);
//     let did = format!("did:pkh:eip155:1:{}", address);
//     let orbit_id = OrbitId::new(
//         did.strip_prefix("did:").unwrap().to_string(),
//         String::from("default"),
//     );
//     let invocation_target = orbit_id.to_resource(
//         Some("kv".to_string()),
//         Some(params.name),
//         Some(params.action),
//     );
//     let invocation = {
//         let context = Contexts::default();
//         let id = URI::String(format!("urn:uuid:{}", Uuid::new_v4()));
//         let property_set = InvProps {
//             invocation_target,
//             expires: None,
//             valid_from: None,
//             extra_fields: None,
//         };
//         KeplerInvocation {
//             context,
//             id,
//             property_set,
//             proof: None,
//         }
//     };
//     let resolver = DID_METHODS.to_resolver();
//     let verification_method = get_verification_method(&did, resolver).await.unwrap();
//     let ldp_options = LinkedDataProofOptions {
//         verification_method: Some(URI::String(verification_method)),
//         proof_purpose: Some(ProofPurpose::CapabilityInvocation),
//         ..Default::default()
//     };
//     let pk: &ethers::core::k256::PublicKey = &wallet.signer().verifying_key().try_into().unwrap();
//     let mut ec_params = ECParams::try_from(pk).unwrap();
//     ec_params.ecc_private_key = Some(Base64urlUInt(wallet.signer().to_bytes().to_vec()));
//     let jwk = JWK::from(Params::EC(ec_params));
//     let proof = LinkedDataProofs::sign(&invocation, &ldp_options, resolver, &jwk, None)
//         .await
//         .unwrap();
//     Json(invocation.set_proof(proof))
// }

#[derive(Serialize, Deserialize)]
struct OrbitParams {
    peer_id: String,
}

async fn create_orbit(
    Json(params): Json<OrbitParams>,
    Extension(user): Extension<Arc<User>>,
) -> Json<KeplerDelegation> {
    let root_cap: UriString = user.orbit_id.to_string().try_into().unwrap();
    let message = Message {
        address: user.wallet.address().try_into().unwrap(),
        chain_id: user.wallet.chain_id(),
        domain: "localhost".try_into().unwrap(),
        issued_at: Utc::now().into(),
        uri: format!("peer:{}", params.peer_id).try_into().unwrap(),
        nonce: generate_nonce(),
        statement: Some("Authorize action (host): Authorize this peer to host your orbit.".into()),
        resources: vec![root_cap.clone(), root_cap],
        version: Version::V1,
        not_before: None,
        expiration_time: None,
        request_id: None,
    };
    let signature = user.wallet.sign_message(message.to_string()).await.unwrap();
    let delegation = cacao_to_zcap(&CACAO::<SignInWithEthereum>::new(
        message.into(),
        BasicSignature::<SIWESignature> {
            s: signature.to_vec().try_into().unwrap(),
        },
    ))
    .unwrap();
    Json(delegation)
}

async fn create_session(Extension(user): Extension<Arc<User>>) -> Json<KeplerDelegation> {
    Json(user.delegation.clone())
}
async fn invoke_session(
    Json(params): Json<InvokeParams>,
    Extension(jwk): Extension<Arc<JWK>>,
    Extension(user): Extension<Arc<User>>,
) -> Json<KeplerInvocation> {
    let did = DID_METHODS
        .generate(&Source::KeyAndPattern(&jwk, "key"))
        .unwrap();
    let did_resolver = DID_METHODS.to_resolver();
    let verification_method = get_verification_method(&did, did_resolver).await.unwrap();
    let invocation_target = user.orbit_id.clone().to_resource(
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

    let ldp_options = LinkedDataProofOptions {
        verification_method: Some(URI::String(verification_method)),
        proof_purpose: Some(ProofPurpose::CapabilityInvocation),
        ..Default::default()
    };
    let resolver = DID_METHODS.to_resolver();
    let mut capability_chain = user
        .delegation
        .proof
        .as_ref()
        .and_then(|proof| proof.property_set.as_ref())
        .and_then(|props| props.get("capabilityChain"))
        .and_then(|chain| chain.as_array().cloned())
        .unwrap_or_default();
    capability_chain.push(serde_json::to_value(&user.delegation).unwrap());

    let mut proof_props = std::collections::HashMap::<String, Value>::new();
    proof_props.insert(
        "capabilityChain".into(),
        serde_json::to_value(capability_chain).unwrap(),
    );

    let proof =
        LinkedDataProofs::sign(&invocation, &ldp_options, resolver, &jwk, Some(proof_props))
            .await
            .unwrap();

    Json(invocation.set_proof(proof))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let jwk = JWK::generate_ed25519().unwrap();
    let wallet = "dcf2cbdd171a21c480aa7f53d77f31bb102282b3ff099c78e3118b37348c72f7"
        .parse::<LocalWallet>()
        .unwrap();
    let user = new_user(wallet, jwk.clone()).await;
    let app = Router::new()
        // .route("/invoke", post(invocation))
        .route("/orbit", post(create_orbit))
        .route("/session/create", post(create_session))
        .route("/session/invoke", post(invoke_session))
        .layer(Extension(Arc::new(user)))
        .layer(Extension(Arc::new(jwk)));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
