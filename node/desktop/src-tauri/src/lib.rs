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

/// Managed state: the in-process executors plus the resolved UC endpoint.
struct AppState {
    hosted: Arc<Hosted>,
    /// Resolved Unity Catalog REST base (the spawned sidecar's dynamic endpoint),
    /// e.g. `http://127.0.0.1:PORT/api/2.1/unity-catalog/`. `None` when UC is
    /// disabled (files run in-memory); the proxy then errors.
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
    message: Vec<u8>,
    headers: Vec<(String, String)>,
    on_chunk: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    let ctx = request_context(&path, headers);
    // The request arrives as proto-encoded bytes and the response chunks are
    // proto too (the Arrow IPC travels as raw binary). `call_server_streaming`
    // uses one codec for both, so the request MUST be Proto, not JSON — sending
    // JSON here yields "failed to decode proto request: unexpected end of
    // buffer". The request is tiny (just the query text), so carrying it as a
    // byte array costs nothing.
    //
    // `call_server_streaming` returns a `'static` future + stream (no router
    // borrow), so it is sound to build under the `State` borrow and await after.
    let fut = state.router(&service)?.call_server_streaming(
        &path,
        ctx,
        Bytes::from(message),
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
///
/// The destination `path` and optional `content_type` ride as request *headers*,
/// not as command-signature args: Tauri resolves signature args from the IPC
/// payload, which here is the raw bytes, so a `path: String` param would fail with
/// "expected a value for key path but the IPC call used a bytes payload". Reading
/// them off `request.headers()` keeps the zero-copy raw body.
#[tauri::command]
async fn files_upload(
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<serde_json::Value, String> {
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let path = header("path").ok_or_else(|| "missing `path` header".to_string())?;
    let content_type = header("content_type");

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

/// JSON response shape for `proxy_request` (matches `ProxyResponse` in
/// tauri-fetch.ts).
#[derive(serde::Serialize)]
struct ProxyResponse {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
}

/// The UI's UC REST base, as the OpenAPI client addresses it. The webview sends
/// requests under this prefix (relative URLs resolve against the app origin); we
/// strip it and re-root the remainder at the spawned sidecar's endpoint.
const UI_UC_PREFIX: &str = "/api/2.1/unity-catalog";

/// Forward a Unity Catalog REST request from the UI to the spawned `uc` sidecar
/// on its dynamic port. A transparent byte proxy: the UI's OpenAPI client is the
/// typed layer, so this hop only needs to re-root the URL and pass method /
/// headers / body / status through. Non-UC URLs are rejected (the JS side only
/// routes UC paths here).
#[tauri::command]
async fn proxy_request(
    state: State<'_, AppState>,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
) -> Result<ProxyResponse, String> {
    let endpoint = state
        .unity_endpoint
        .as_deref()
        .ok_or("Unity Catalog is not running")?;

    // The incoming URL may be absolute (webview origin) or relative
    // (`/api/2.1/unity-catalog/...`, the UI's relative baseUrl). Parse against a
    // dummy base so both forms work, then extract the path + query and re-root
    // under the sidecar endpoint.
    let base = reqwest::Url::parse("http://localhost").unwrap();
    let parsed = base
        .join(&url)
        .map_err(|e| format!("bad proxy url {url}: {e}"))?;
    let path = parsed.path();
    let rel = path
        .strip_prefix(UI_UC_PREFIX)
        .ok_or_else(|| format!("not a Unity Catalog path: {path}"))?
        .trim_start_matches('/');
    // `endpoint` ends with `/api/2.1/unity-catalog/`; append the relative path + query.
    let mut target = format!("{}{rel}", endpoint.trim_end_matches('/').to_string() + "/");
    if let Some(q) = parsed.query() {
        target.push('?');
        target.push_str(q);
    }

    let client = reqwest::Client::new();
    let verb = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|e| format!("bad method {method}: {e}"))?;
    let mut req = client.request(verb, &target);
    for (k, v) in headers {
        // Skip hop-by-hop / origin headers that don't apply to the re-rooted call.
        let lk = k.to_ascii_lowercase();
        if lk == "host" || lk == "origin" || lk == "content-length" {
            continue;
        }
        req = req.header(k, v);
    }
    if !body.is_empty() {
        req = req.body(body);
    }

    let resp = req.send().await.map_err(|e| format!("uc proxy: {e}"))?;
    let status = resp.status().as_u16();
    let resp_headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let resp_body = resp.text().await.map_err(|e| format!("uc proxy body: {e}"))?;
    Ok(ProxyResponse {
        status,
        body: resp_body,
        headers: resp_headers,
    })
}

/// The managed child handle for the spawned UC server, so the exit hook can kill
/// it. `None` once taken/killed.
struct UcSidecar(std::sync::Mutex<Option<tauri_plugin_shell::process::CommandChild>>);

/// The in-repo data directory for the UC server (gitignored): SQLite DB + config
/// live here so they survive restarts and are inspectable in the working tree.
fn uc_data_dir() -> std::path::PathBuf {
    // .../node/desktop/src-tauri/../.uc-data → node/desktop/.uc-data
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.uc-data")
}

/// Write the UC server config (SQLite + local `file://` managed storage, all
/// under `.uc-data/`) and return its path. We supply the config explicitly
/// because a provided config file with no `encryption` block deserializes to
/// `None` (the dev-KEK default only applies to a config-LESS launch), which the
/// server rejects — so we include the dev KEK.
///
/// `managed_storage_root` is `file://<.uc-data/storage>` so catalog data persists
/// on disk (inspectable). A managed catalog requires a resolvable storage root,
/// and the local `file://` root must be covered by `local_storage.allowed-roots`
/// (deny-by-default governance). NOTE the key casing — this is the easy trap:
/// `Config` fields are snake_case (`local_storage`, `managed_storage_root`) but
/// the nested `LocalStorageConfig` is kebab-case (`allowed-roots`). Using the
/// wrong case silently drops the allow-root → "local storage is not enabled".
fn write_uc_config(data_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("creating {data_dir:?}: {e}"))?;
    let canonical = std::fs::canonicalize(data_dir).map_err(|e| e.to_string())?;
    let db_path = canonical.join("catalog.db");
    let storage_root = canonical.join("storage");
    std::fs::create_dir_all(&storage_root).map_err(|e| e.to_string())?;

    let config = format!(
        "host: 127.0.0.1\n\
         port: 0\n\
         backend:\n\
         \x20\x20engine: sqlite\n\
         \x20\x20path: {db}\n\
         encryption:\n\
         \x20\x20active:\n\
         \x20\x20\x20\x20id: dev\n\
         \x20\x20\x20\x20key: AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=\n\
         local_storage:\n\
         \x20\x20allowed-roots:\n\
         \x20\x20\x20\x20- {root}\n\
         managed_storage_root: \"file://{root}\"\n",
        db = db_path.display(),
        root = storage_root.display(),
    );
    let config_path = canonical.join("config.yaml");
    std::fs::write(&config_path, config).map_err(|e| format!("writing config: {e}"))?;
    Ok(config_path)
}

/// Spawn the local `uc` server (SQLite, port 0), scrape the bound port from its
/// startup line, and return the REST endpoint. Stores the child in managed state
/// so the exit hook can kill it.
async fn spawn_uc_sidecar(app: &tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_shell::ShellExt;
    use tauri_plugin_shell::process::CommandEvent;

    let config_path = write_uc_config(&uc_data_dir())?;

    // Spawn as a Tauri sidecar: the binary lives at
    // `src-tauri/binaries/uc-server-<target-triple>` (declared in tauri.conf.json
    // `externalBin`; `just uc-setup` symlinks the sibling build there). Tauri
    // resolves the triple-suffixed path itself, so we avoid the shell scope's
    // `cmd`-string path resolution entirely.
    let (mut rx, child) = app
        .shell()
        .sidecar("uc-server")
        .map_err(|e| format!("uc-server sidecar not found (run `just uc-setup`?): {e}"))?
        .args([
            "server",
            "--config",
            &config_path.to_string_lossy(),
            "--port",
            "0",
            "--quiet",
        ])
        .spawn()
        .map_err(|e| format!("failed to spawn uc server: {e}"))?;

    app.manage(UcSidecar(std::sync::Mutex::new(Some(child))));

    // The server prints `✅listening on http://<host>:<port>` (status::success);
    // colors are auto-disabled when piped. Scrape the address from that line,
    // tolerating an emoji/ANSI prefix.
    while let Some(event) = rx.recv().await {
        let line = match event {
            CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                String::from_utf8_lossy(&bytes).into_owned()
            }
            CommandEvent::Terminated(payload) => {
                return Err(format!(
                    "uc server exited before announcing its address (code {:?})",
                    payload.code
                ));
            }
            _ => continue,
        };
        if let Some(addr) = parse_uc_addr(&line) {
            let endpoint = format!("http://{addr}/api/2.1/unity-catalog/");
            // First-run seed so the catalog browser + IntelliSense have data.
            if let Err(e) = seed_uc(&endpoint).await {
                eprintln!("[uc] seed skipped: {e}");
            }
            return Ok(endpoint);
        }
    }
    Err("uc server stream ended before announcing its address".into())
}

/// First-run seed: if the catalog is empty, create a default `main.default` so
/// the catalog browser and SQL IntelliSense have something to show. Idempotent —
/// skips if any catalog already exists (so it doesn't clobber user-created data
/// across restarts). `endpoint` ends with `/api/2.1/unity-catalog/`.
async fn seed_uc(endpoint: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let base = endpoint.trim_end_matches('/');

    let existing: serde_json::Value = client
        .get(format!("{base}/catalogs"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let has_catalogs = existing
        .get("catalogs")
        .and_then(|c| c.as_array())
        .is_some_and(|a| !a.is_empty());
    if has_catalogs {
        return Ok(());
    }

    client
        .post(format!("{base}/catalogs"))
        .json(&serde_json::json!({ "name": "main", "comment": "Default catalog" }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| format!("create catalog: {e}"))?;
    client
        .post(format!("{base}/schemas"))
        .json(&serde_json::json!({ "name": "default", "catalog_name": "main" }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| format!("create schema: {e}"))?;
    eprintln!("[uc] seeded catalog main.default");
    Ok(())
}

/// Extract `host:port` from a `listening on http://host:port` startup line.
fn parse_uc_addr(line: &str) -> Option<String> {
    let idx = line.find("http://")?;
    let rest = &line[idx + "http://".len()..];
    // Cut at the first char that can't be part of `host:port` (whitespace, slash,
    // or a trailing ANSI escape).
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '/' || c == '\u{1b}')
        .unwrap_or(rest.len());
    let addr = rest[..end].trim();
    if addr.contains(':') {
        Some(addr.to_string())
    } else {
        None
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // Resolve the UC endpoint. `OPEN_LAKEHOUSE_UC_URL` (incl. empty for
            // "no UC → in-memory files") takes precedence; otherwise spawn and
            // manage the local `uc` server (SQLite persisted under .uc-data/).
            let unity_endpoint = match std::env::var("OPEN_LAKEHOUSE_UC_URL") {
                Ok(url) if url.is_empty() => {
                    eprintln!("[uc] OPEN_LAKEHOUSE_UC_URL is empty → files in-memory, no UC");
                    None
                }
                Ok(url) => {
                    eprintln!("[uc] using OPEN_LAKEHOUSE_UC_URL={url}");
                    Some(url)
                }
                Err(_) => {
                    eprintln!("[uc] spawning local uc sidecar…");
                    let handle = app.handle().clone();
                    match tauri::async_runtime::block_on(spawn_uc_sidecar(&handle)) {
                        Ok(endpoint) => {
                            eprintln!("uc server listening at {endpoint}");
                            Some(endpoint)
                        }
                        Err(e) => {
                            // Don't abort the app: fall back to the in-memory file
                            // store so the editor still runs without UC.
                            eprintln!("uc sidecar failed, files in-memory: {e}");
                            None
                        }
                    }
                }
            };

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
            proxy_request,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Kill the spawned UC server when the app exits.
            if let tauri::RunEvent::Exit = event {
                if let Some(sidecar) = app.try_state::<UcSidecar>() {
                    if let Some(child) = sidecar.0.lock().unwrap().take() {
                        let _ = child.kill();
                    }
                }
            }
        });
}
