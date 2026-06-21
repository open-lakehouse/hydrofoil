//! Open Lakehouse desktop shell (Tauri v2).
//!
//! For now this is a thin native wrapper around the existing web UI. The single
//! `proxy_request` command below is the future seam for serving selected UI
//! requests locally (e.g. by reusing the `portal` crate) instead of routing them
//! to the HTTP server. It is currently inert: the JS side
//! (node/desktop/src/tauri-fetch.ts) never invokes it, falling back to HTTP for
//! everything, so this command exists and is registered but is never called.

use serde_json::Value;

/// Future seam for handling a UI request inside the desktop backend.
///
/// Not yet implemented — the frontend's `shouldRouteThroughRust` returns false,
/// so every request falls through to HTTP and this command is never invoked.
/// When bridging a real service, parse the request here (or dispatch to a reused
/// crate such as `portal`) and return a `{ status, body, headers }` payload.
#[tauri::command]
async fn proxy_request(
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
) -> Result<Value, String> {
    let _ = (method, url, headers, body);
    Err("proxy_request not implemented (HTTP fallback in use)".into())
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![proxy_request])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
