mod host;
mod serde_siwe;
mod session;
mod util;
mod zcap;

use std::future::Future;

use js_sys::Promise;
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
            .map(session::complete_session_setup)
            .and_then(|session| {
                serde_json::to_string(&session).map_err(session::Error::JSONSerializing)
            }),
    )
}

#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn invoke(session: String, path: String, action: String) -> Promise {
    map_async_jsvalue(async move {
        zcap::InvocationHeaders::from(
            serde_json::from_str(&session).map_err(zcap::Error::JSONDeserializing)?,
            path,
            action,
        )
        .await
        .and_then(|headers| serde_json::to_string(&headers).map_err(zcap::Error::JSONSerializing))
    })
}

#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn generateHostSIWEMessage(config: String) -> Result<String, JsValue> {
    map_jsvalue(
        serde_json::from_str(&config)
            .map_err(host::Error::JSONDeserializing)
            .and_then(host::generate_host_siwe_message)
            .map(|message| message.to_string()),
    )
}

#[wasm_bindgen]
#[allow(non_snake_case)]
pub fn host(signedSIWEMessage: String) -> Result<String, JsValue> {
    map_jsvalue(
        serde_json::from_str(&signedSIWEMessage)
            .map_err(host::Error::JSONDeserializing)
            .map(host::host)
            .and_then(|headers| {
                serde_json::to_string(&headers).map_err(host::Error::JSONSerializing)
            }),
    )
}
