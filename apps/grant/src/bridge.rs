//! Bindings to `js/bridge.js`: the wallet, JSON-RPC and time.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/js/bridge.js")]
extern "C" {
    #[wasm_bindgen(js_name = walletNames)]
    fn wallet_names_js() -> String;

    #[wasm_bindgen(js_name = connect, catch)]
    async fn connect_js(name: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = disconnect, catch)]
    async fn disconnect_js() -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = signTransaction, catch)]
    async fn sign_transaction_js(tx: &[u8]) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = rpc, catch)]
    async fn rpc_js(url: &str, method: &str, params: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = sleep)]
    async fn sleep_js(ms: u32);

    #[wasm_bindgen(js_name = nowSecs)]
    pub fn now_secs() -> i64;

    #[wasm_bindgen(js_name = isoTime)]
    pub fn iso_time(secs: i64) -> String;
}

fn message(err: &JsValue) -> String {
    err.dyn_ref::<js_sys::Error>().map_or_else(
        || err.as_string().unwrap_or_else(|| format!("{err:?}")),
        |e| String::from(e.message()),
    )
}

pub fn wallet_names() -> Vec<String> {
    serde_json::from_str(&wallet_names_js()).unwrap_or_default()
}

pub async fn connect(name: &str) -> Result<String, String> {
    connect_js(name)
        .await
        .map_err(|e| message(&e))?
        .as_string()
        .ok_or_else(|| "the wallet returned no address".into())
}

pub async fn disconnect() -> Result<(), String> {
    disconnect_js().await.map(|_| ()).map_err(|e| message(&e))
}

pub async fn sign_transaction(tx: &[u8]) -> Result<Vec<u8>, String> {
    let signed = sign_transaction_js(tx).await.map_err(|e| message(&e))?;
    signed
        .dyn_ref::<js_sys::Uint8Array>()
        .map(js_sys::Uint8Array::to_vec)
        .ok_or_else(|| "the wallet returned something other than bytes".into())
}

pub async fn rpc(
    url: &str,
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let out = rpc_js(url, method, &params.to_string())
        .await
        .map_err(|e| message(&e))?;
    let text = out.as_string().ok_or("RPC returned no text")?;
    serde_json::from_str(&text).map_err(|e| format!("RPC {method}: {e}"))
}

pub async fn sleep(ms: u32) {
    sleep_js(ms).await;
}
