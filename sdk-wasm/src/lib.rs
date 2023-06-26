mod definitions;

use js_sys::Promise;
use kepler_sdk::*;
use std::future::Future;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

fn map_jsvalue<E: std::error::Error>(result: Result<String, E>) -> Result<String, JsValue> {
    match result {
        Ok(string) => Ok(string),
        Err(err) => Err(err.to_string().into()),
    }
}

fn map_async_jsvalue<E: std::error::Error>(
    future: impl Future<Output = Result<String, E>> + 'static,
) -> Promise {
    future_to_promise(async {
        match future.await {
            Ok(string) => Ok(string.into()),
            Err(err) => Err(err.to_string().into()),
        }
    })
}

// removing since we have duplicate usage elsewhere
// #[wasm_bindgen]
// #[allow(non_snake_case)]
// /// Initialise console-error-panic-hook to improve debug output for panics.
// ///
// /// Run once on initialisation.
// pub fn initPanicHook() {
//     console_error_panic_hook::set_once();
// }

#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn makeOrbitId(address: String, chainId: u32, name: Option<String>) -> String {
    util::make_orbit_id_pkh_eip155(address, chainId, name)
}

#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn prepareSession(config: String) -> Promise {
    map_async_jsvalue(async move {
        session::prepare_session(
            serde_json::from_str(&config).map_err(session::Error::JSONSerializing)?,
        )
        .await
        .and_then(|preparation| {
            serde_json::to_string(&preparation).map_err(session::Error::JSONDeserializing)
        })
    })
}

#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn completeSessionSetup(config: String) -> Result<String, JsValue> {
    map_jsvalue(
        serde_json::from_str(&config)
            .map_err(session::Error::JSONDeserializing)
            .and_then(session::complete_session_setup)
            .and_then(|session| {
                serde_json::to_string(&session).map_err(session::Error::JSONSerializing)
            }),
    )
}

#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn invoke(session: String, service: String, path: String, action: String) -> Promise {
    map_async_jsvalue(async move {
        authorization::InvocationHeaders::from(
            serde_json::from_str(&session).map_err(authorization::Error::JSONDeserializing)?,
            vec![(service, path, action)],
        )
        .await
        .and_then(|headers| {
            serde_json::to_string(&headers).map_err(authorization::Error::JSONSerializing)
        })
    })
}

#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn generateHostSIWEMessage(config: String) -> Result<String, JsValue> {
    map_jsvalue(
        serde_json::from_str(&config)
            .map_err(siwe_utils::Error::JSONDeserializing)
            .and_then(siwe_utils::generate_host_siwe_message)
            .map(|message| message.to_string()),
    )
}

#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn siweToDelegationHeaders(signedSIWEMessage: String) -> Result<String, JsValue> {
    map_jsvalue(
        serde_json::from_str(&signedSIWEMessage)
            .map_err(siwe_utils::Error::JSONDeserializing)
            .map(siwe_utils::siwe_to_delegation_headers)
            .and_then(|headers| {
                serde_json::to_string(&headers).map_err(siwe_utils::Error::JSONSerializing)
            }),
    )
}
