use axum::{routing::post, Extension, Json, Router};
use ethers::{
    core::utils::to_checksum,
    prelude::rand::{prelude::StdRng, SeedableRng},
    signers::{LocalWallet, Signer},
};
use kepler_lib::{cacaos::siwe::TimeStamp, resource::OrbitId, ssi::jwk::JWK};
use kepler_sdk::{
    authorization::{DelegationHeaders, InvocationHeaders},
    session::{complete_session_setup, prepare_session, Session, SessionConfig, SignedSession},
    siwe_utils::{
        generate_host_siwe_message, siwe_to_delegation_headers, HostConfig, SignedMessage,
    },
};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, str::FromStr, sync::Arc};

struct User {
    wallet: LocalWallet,
    session: Session,
    session_config: SessionConfig,
}

async fn new_user(wallet: LocalWallet, jwk: JWK) -> User {
    let address = to_checksum(&wallet.address(), None);
    let did = format!("did:pkh:eip155:1:{}", address);
    let orbit_id = OrbitId::new(
        did.strip_prefix("did:").unwrap().to_string(),
        String::from("default"),
    );

    let session_config = SessionConfig {
        actions: [(
            "".into(),
            vec![
                "put".into(),
                "get".into(),
                "del".into(),
                "metadata".into(),
                "list".into(),
            ],
        )]
        .into(),
        service: "kv".to_string(),
        address: wallet.address().into(),
        chain_id: 1,
        domain: "localhost".try_into().unwrap(),
        orbit_id,
        not_before: None,
        parents: None,
        jwk: Some(jwk),
        issued_at: TimeStamp::from_str("1985-04-12T23:20:50.52Z").unwrap(),
        expiration_time: TimeStamp::from_str("2985-04-12T23:20:50.52Z").unwrap(),
    };
    let prepared_session = prepare_session(session_config.clone()).await.unwrap();
    let signature = wallet
        .sign_message(prepared_session.siwe.to_string())
        .await
        .unwrap();
    let session = complete_session_setup(SignedSession {
        session: prepared_session,
        signature: signature.to_vec().try_into().unwrap(),
    })
    .unwrap();

    User {
        wallet,
        session,
        session_config,
    }
}

#[derive(Serialize, Deserialize)]
struct InvokeParams {
    name: String,
    action: String,
}

#[derive(Serialize, Deserialize)]
struct OrbitParams {
    peer_id: String,
}

async fn create_orbit(
    Json(params): Json<OrbitParams>,
    Extension(user): Extension<Arc<User>>,
) -> Json<DelegationHeaders> {
    let message = generate_host_siwe_message(HostConfig {
        address: user.session_config.address,
        chain_id: user.session_config.chain_id,
        domain: user.session_config.domain.clone(),
        issued_at: user.session_config.issued_at.clone(),
        orbit_id: user.session_config.orbit_id.clone(),
        peer_id: params.peer_id,
    })
    .unwrap();
    let signature = user.wallet.sign_message(message.to_string()).await.unwrap();
    let delegation = siwe_to_delegation_headers(SignedMessage {
        siwe: message,
        signature: signature.to_vec().try_into().unwrap(),
    });
    Json(delegation)
}

async fn create_session(Extension(user): Extension<Arc<User>>) -> Json<DelegationHeaders> {
    Json(user.session.delegation_header.clone())
}
async fn invoke_session(
    Json(params): Json<InvokeParams>,
    Extension(user): Extension<Arc<User>>,
) -> Json<InvocationHeaders> {
    let headers = InvocationHeaders::from(user.session.clone(), params.name, params.action)
        .await
        .unwrap();
    Json(headers)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let jwk = JWK::generate_ed25519().unwrap();
    let id = 0_u128.to_ne_bytes();
    let mut seed = id.to_vec();
    seed.extend_from_slice(&id);
    let mut rng = StdRng::from_seed(seed.try_into().unwrap());
    let wallet = LocalWallet::new(&mut rng);
    let user = new_user(wallet, jwk.clone()).await;
    let app = Router::new()
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
