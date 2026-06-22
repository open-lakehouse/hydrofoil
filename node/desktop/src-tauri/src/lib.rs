//! Open Lakehouse desktop shell (Tauri v2).
//!
//! The desktop app runs the portal (Tags + Files) and hydrofoil (QueryService)
//! executors **in-process** instead of over HTTP, so a local run needs no Docker
//! Compose stack for those services. Only Unity Catalog must be a real server,
//! reached over HTTP — run as a Tauri sidecar when bundled, or pointed at a dev
//! UC via `OPEN_LAKEHOUSE_UC_URL`. Heavier services (Lineage, MLflow) stay in
//! Compose.
//!
//! The UI reaches the executors through Tauri commands (see `tauri-transport.ts`
//! / the Files host seam on the JS side):
//!   - `connect_unary` / `connect_stream` drive the `connectrpc::Router`
//!     dispatchers for **Tags** and the **QueryService** — JSON in, JSON out
//!     (unary), or raw Connect frames over a `Channel` (server-streaming).
//!   - `files_*` call the `FileStore` directly with native types (no proto
//!     framing) — the store already is the sanitized handler.

use std::sync::Arc;

use bytes::Bytes;
use connectrpc::{CodecFormat, Dispatcher, Payload, RequestContext};
use desktop_host::{HostConfig, Hosted};
use futures::StreamExt;
use http::HeaderMap;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Manager, State};

/// Default Unity Catalog REST endpoint for local dev (the `scripts/dev-desktop.sh`
/// UC server). Overridable with `OPEN_LAKEHOUSE_UC_URL`.
const DEFAULT_UC_URL: &str = "http://localhost:8080/api/2.1/unity-catalog/";

/// Managed state: the in-process executors plus the resolved UC endpoint.
struct AppState {
    hosted: Arc<Hosted>,
    #[allow(dead_code)] // surfaced to commands/diagnostics; not yet read directly
    unity_endpoint: Option<String>,
}

impl AppState {
    /// Select the router that owns a service group: `"tags"` (portal Tags) or
    /// `"query"` (hydrofoil QueryService). Files is not a router — it is served by
    /// the `files_*` commands directly.
    fn router(&self, service: &str) -> Result<&connectrpc::Router, String> {
        match service {
            "tags" => Ok(&self.hosted.tags),
            "query" => Ok(&self.hosted.query),
            other => Err(format!("unknown service group: {other}")),
        }
    }

    /// Clone the file-store handle out of managed state so commands can drop the
    /// `State` borrow before awaiting the store call.
    fn files(&self) -> Arc<dyn portal::store::FileStore> {
        Arc::clone(&self.hosted.files)
    }
}

/// Build a [`RequestContext`] from the JS-supplied header pairs, pinning the
/// method path so the dispatcher and any header-derived identity see it.
fn request_context(path: &str, headers: Vec<(String, String)>) -> RequestContext {
    let mut map = HeaderMap::new();
    for (k, v) in headers {
        if let (Ok(name), Ok(value)) = (
            http::HeaderName::try_from(k.as_str()),
            http::HeaderValue::try_from(v.as_str()),
        ) {
            map.insert(name, value);
        }
    }
    RequestContext::new(map).with_path(path)
}

/// Generic unary Connect call: JSON request → JSON response, dispatched in-process
/// against the selected router. Serves portal Tags (and any future unary RPCs).
#[tauri::command]
async fn connect_unary(
    state: State<'_, AppState>,
    service: String,
    path: String,
    message: String,
    headers: Vec<(String, String)>,
) -> Result<String, String> {
    let ctx = request_context(&path, headers);
    // `call_unary` returns a `'static` future (it does not borrow the router), so
    // building it under the `State` borrow and awaiting it afterwards is sound.
    let fut = state.router(&service)?.call_unary(
        &path,
        ctx,
        Payload::new(Bytes::from(message.into_bytes()), CodecFormat::Json),
        CodecFormat::Json,
    );
    let resp = fut.await.map_err(|e| e.to_string())?;
    String::from_utf8(resp.body.to_vec()).map_err(|e| format!("non-UTF8 JSON response: {e}"))
}

/// Generic server-streaming Connect call. Drives the dispatcher's response stream
/// and forwards each Connect frame to the UI over a `Channel` as raw bytes.
///
/// Used for `QueryService/RunQuery`: chunks are encoded with the **Proto** codec so
/// the Arrow IPC payload travels as raw binary (no JSON byte-array bloat); the JS
/// transport decodes them with the generated binary codec, matching the web build's
/// `useBinaryFormat`. The stream's end is signaled by this command's promise
/// resolving (Channel delivery is ordered, and the command returns only after every
/// `send`).
#[tauri::command]
async fn connect_stream(
    state: State<'_, AppState>,
    service: String,
    path: String,
    message: String,
    headers: Vec<(String, String)>,
    on_chunk: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    let ctx = request_context(&path, headers);
    // `call_server_streaming` returns a `'static` future + stream (no router
    // borrow), so it is sound to build under the `State` borrow and await after.
    let fut = state.router(&service)?.call_server_streaming(
        &path,
        ctx,
        Bytes::from(message.into_bytes()),
        CodecFormat::Proto,
    );
    let response = fut.await.map_err(|e| e.to_string())?;

    let mut stream = response.body;
    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|e| e.to_string())?;
        on_chunk
            .send(InvokeResponseBody::Raw(chunk.to_vec()))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// --- Files: direct FileStore calls (no dispatcher, native types) ---------------

/// File/directory metadata commands return the store's domain types directly as
/// JSON (the buffa-generated messages derive serde). Map store errors to a string.
#[tauri::command]
async fn files_stat(state: State<'_, AppState>, path: String) -> Result<serde_json::Value, String> {
    let files = state.files();
    let meta = files.stat_file(&path).await.map_err(|e| e.to_string())?;
    serde_json::to_value(meta).map_err(|e| e.to_string())
}

#[tauri::command]
async fn files_list(
    state: State<'_, AppState>,
    path: String,
    max_results: Option<i64>,
    page_token: Option<String>,
) -> Result<serde_json::Value, String> {
    let page = portal::store::Page {
        max_results: max_results.map(|n| n.max(0) as usize),
        page_token,
    };
    let files = state.files();
    let (contents, next_page_token) = files
        .list_directory(&path, page)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(serde_json::json!({
        "contents": contents,
        "next_page_token": next_page_token,
    }))
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn files_create_dir(
    state: State<'_, AppState>,
    path: String,
) -> Result<serde_json::Value, String> {
    let files = state.files();
    let meta = files
        .create_directory(&path)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(meta).map_err(|e| e.to_string())
}

#[tauri::command]
async fn files_delete(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let files = state.files();
    files.delete_file(&path).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn files_delete_dir(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let files = state.files();
    files
        .delete_directory(&path)
        .await
        .map_err(|e| e.to_string())
}

/// Stream a file's bytes to the UI over a `Channel` as raw chunks — backed by the
/// store's chunked GET, never buffering the whole file. End-of-stream is the
/// command's promise resolving.
#[tauri::command]
async fn files_download(
    state: State<'_, AppState>,
    path: String,
    offset: Option<i64>,
    length: Option<i64>,
    on_chunk: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    let files = state.files();
    let mut stream = files
        .read_file_stream(&path, offset, length)
        .await
        .map_err(|e| e.to_string())?;
    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|e| e.to_string())?;
        on_chunk
            .send(InvokeResponseBody::Raw(chunk.to_vec()))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Upload a file's bytes (sent as a single raw body from JS) into the store via a
/// streaming put — the bytes are handed to `put_file_stream` as one chunk, so the
/// store still does a (multipart) streaming upload without the desktop path ever
/// reconstructing Connect `StreamMessage` framing.
#[tauri::command]
async fn files_upload(
    state: State<'_, AppState>,
    path: String,
    content_type: Option<String>,
    request: tauri::ipc::Request<'_>,
) -> Result<serde_json::Value, String> {
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("upload body must be raw bytes".into());
    };
    let bytes = Bytes::copy_from_slice(data);
    let chunks: portal::store::ByteStream =
        Box::pin(futures::stream::once(async move { Ok(bytes) }));
    let files = state.files();
    let meta = files
        .put_file_stream(&path, content_type, chunks)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(meta).map_err(|e| e.to_string())
}

/// Resolve the Unity Catalog endpoint: `OPEN_LAKEHOUSE_UC_URL` if set, else the
/// dev default. (When a UC sidecar binary is bundled, the spawn path overrides
/// this with the sidecar's chosen port — see [`spawn_uc_sidecar`].)
fn resolve_uc_endpoint() -> Option<String> {
    match std::env::var("OPEN_LAKEHOUSE_UC_URL") {
        Ok(url) if !url.is_empty() => Some(url),
        _ => Some(DEFAULT_UC_URL.to_string()),
    }
}

/// Spawn a bundled Unity Catalog sidecar (`binaries/uc-server`), scrape the port
/// it binds from its stdout, and return the resolved REST endpoint. The child
/// handle is stored in managed state so [`run`]'s exit hook can kill it.
///
/// SCAFFOLD: not yet invoked — no UC binary is bundled. To enable, add
/// `bundle.externalBin: ["binaries/uc-server"]` to `tauri.conf.json`, drop the
/// target-triple-suffixed binary under `src-tauri/binaries/`, and call this from
/// `setup()` instead of [`resolve_uc_endpoint`]. The capability already allows
/// the spawn (`shell:allow-spawn` for `binaries/uc-server`).
#[allow(dead_code)]
async fn spawn_uc_sidecar(app: &tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_shell::ShellExt;
    use tauri_plugin_shell::process::CommandEvent;

    let (mut rx, child) = app
        .shell()
        .sidecar("uc-server")
        .map_err(|e| e.to_string())?
        .args(["--port", "0"])
        .spawn()
        .map_err(|e| format!("failed to spawn uc-server sidecar: {e}"))?;

    // Hold the child so the exit hook can kill it.
    app.manage(std::sync::Mutex::new(Some(child)));

    // Wait for the sidecar to announce its port on stdout (expects a line like
    // `listening on http://127.0.0.1:<port>` — adjust to the real UC output).
    while let Some(event) = rx.recv().await {
        if let CommandEvent::Stdout(bytes) = event {
            let line = String::from_utf8_lossy(&bytes);
            if let Some(port) = parse_uc_port(&line) {
                return Ok(format!("http://127.0.0.1:{port}/api/2.1/unity-catalog/"));
            }
        }
    }
    Err("uc-server exited before announcing a port".into())
}

/// Parse the port from a UC sidecar stdout line. SCAFFOLD: match the real output.
#[allow(dead_code)]
fn parse_uc_port(line: &str) -> Option<u16> {
    line.rsplit(':')
        .next()
        .and_then(|tail| tail.trim().split('/').next())
        .and_then(|s| s.trim().parse().ok())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // TODO(uc-sidecar): when a `binaries/uc-server` sidecar is bundled,
            // spawn it here, scrape its `--port` from stdout, store the child in
            // managed state, and override the endpoint below. Until then we point
            // at OPEN_LAKEHOUSE_UC_URL / the dev UC. Kill the child on
            // RunEvent::Exit (see `run()` tail when wired).
            let unity_endpoint = resolve_uc_endpoint();

            let cfg = HostConfig {
                unity_endpoint: unity_endpoint.clone(),
                ..Default::default()
            };

            // Build the in-process executors on the Tokio runtime Tauri runs
            // setup on. block_on is fine here: startup is allowed to wait for the
            // engine + UC factory to initialize before the window serves requests.
            let hosted = tauri::async_runtime::block_on(desktop_host::build(cfg))
                .map_err(|e| format!("failed to build in-process services: {e}"))?;

            app.manage(AppState {
                hosted: Arc::new(hosted),
                unity_endpoint,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            connect_unary,
            connect_stream,
            files_stat,
            files_list,
            files_create_dir,
            files_delete,
            files_delete_dir,
            files_download,
            files_upload,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
