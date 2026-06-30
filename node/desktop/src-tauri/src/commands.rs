//! The Tauri command surface the webview invokes.
//!
//! Three kinds of command live here:
//!   - `connect_*` / `query_ingest` drive the in-process [`connectrpc::Router`]
//!     dispatchers for **Tags**, the **QueryService** and the **IngestService**.
//!   - `files_*` call the [`FileStore`](portal::store::FileStore) directly with
//!     native types (no proto framing) — the store already is the sanitized handler.
//!   - the environment-management commands (list/create/start/stop, module key and
//!     observability config) orchestrate [`crate::env`], [`crate::uc`],
//!     [`crate::modules`] and [`crate::telemetry`].
//!
//! The UC REST proxy (`proxy_request`) and notebook commands are registered from
//! their own modules ([`crate::uc`], [`crate::notebook`]).

use bytes::Bytes;
use connectrpc::{CodecFormat, Dispatcher, Payload, RequestContext};
use futures::StreamExt;
use http::HeaderMap;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Manager, State};

use crate::env::{
    active_environment_descriptor, allocate_env_id, read_environments, write_environments,
    AppState, Environment,
};
use crate::kek;
use crate::modules;
use crate::notebook::Notebooks;
use crate::paths::{env_home_dir, env_uc_dir};
use crate::supervisor::Supervisor;
use crate::telemetry::{self, Telemetry};
use crate::topology;
use crate::uc;

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
pub(crate) async fn connect_unary(
    state: State<'_, AppState>,
    service: String,
    path: String,
    message: String,
    headers: Vec<(String, String)>,
) -> Result<String, String> {
    let ctx = request_context(&path, headers);
    let active = state.snapshot()?;
    // `call_unary` returns a `'static` future (it does not borrow the router), so
    // building it under the snapshot and awaiting it afterwards is sound.
    let fut = active.router(&service)?.call_unary(
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
pub(crate) async fn connect_stream(
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
    // borrow), so it is sound to build under the snapshot and await after.
    let active = state.snapshot()?;
    let fut = active.router(&service)?.call_server_streaming(
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

/// Unary Connect call with the **Proto** codec, for RPCs whose request/response
/// carry binary payloads that JSON would bloat (base64). Used by
/// `IngestService/PreviewFile`, whose response carries Arrow IPC bytes. Request
/// and response are proto-encoded message bytes.
#[tauri::command]
pub(crate) async fn connect_unary_proto(
    state: State<'_, AppState>,
    service: String,
    path: String,
    message: Vec<u8>,
    headers: Vec<(String, String)>,
) -> Result<Vec<u8>, String> {
    let ctx = request_context(&path, headers);
    let active = state.snapshot()?;
    let fut = active.router(&service)?.call_unary(
        &path,
        ctx,
        Payload::new(Bytes::from(message), CodecFormat::Proto),
        CodecFormat::Proto,
    );
    let resp = fut.await.map_err(|e| e.to_string())?;
    Ok(resp.body.to_vec())
}

/// Client-streaming Connect call: the UI sends the request frames as an ordered
/// list of proto-encoded message bytes (one `IngestTableRequest` each), and the
/// handler returns a single proto-encoded response. Used by
/// `IngestService/IngestTable`.
///
/// The dispatcher's `RequestStream` yields each message's *decoded* payload bytes
/// (the codec then decodes each), so we hand it the frames verbatim — no envelope
/// framing. On desktop the bulk data rides via the first frame's `source_path`
/// (the host reads the file), so the frame list is small; the streaming RPC shape
/// is kept so the same handler serves a future web client that streams Arrow IPC.
#[tauri::command]
pub(crate) async fn query_ingest(
    state: State<'_, AppState>,
    service: String,
    path: String,
    frames: Vec<Vec<u8>>,
    headers: Vec<(String, String)>,
) -> Result<Vec<u8>, String> {
    use connectrpc::ConnectError;

    let ctx = request_context(&path, headers);
    let active = state.snapshot()?;
    let requests: connectrpc::dispatcher::RequestStream = Box::pin(futures::stream::iter(
        frames
            .into_iter()
            .map(|f| Ok::<Bytes, ConnectError>(Bytes::from(f))),
    ));
    let fut =
        active
            .router(&service)?
            .call_client_streaming(&path, ctx, requests, CodecFormat::Proto);
    let resp = fut.await.map_err(|e| e.to_string())?;
    Ok(resp.body.to_vec())
}

// --- Files: direct FileStore calls (no dispatcher, native types) ---------------

/// File/directory metadata commands return the store's domain types directly as
/// JSON (the buffa-generated messages derive serde). Map store errors to a string.
#[tauri::command]
pub(crate) async fn files_stat(
    state: State<'_, AppState>,
    path: String,
) -> Result<serde_json::Value, String> {
    let files = state.snapshot()?.files()?;
    let meta = files.stat_file(&path).await.map_err(|e| e.to_string())?;
    serde_json::to_value(meta).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn files_list(
    state: State<'_, AppState>,
    path: String,
    max_results: Option<i64>,
    page_token: Option<String>,
) -> Result<serde_json::Value, String> {
    let page = portal::store::Page {
        max_results: max_results.map(|n| n.max(0) as usize),
        page_token,
    };
    let files = state.snapshot()?.files()?;
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
pub(crate) async fn files_create_dir(
    state: State<'_, AppState>,
    path: String,
) -> Result<serde_json::Value, String> {
    let files = state.snapshot()?.files()?;
    let meta = files
        .create_directory(&path)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(meta).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn files_delete(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let files = state.snapshot()?.files()?;
    files.delete_file(&path).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn files_delete_dir(
    state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let files = state.snapshot()?.files()?;
    files
        .delete_directory(&path)
        .await
        .map_err(|e| e.to_string())
}

/// Stream a file's bytes to the UI over a `Channel` as raw chunks — backed by the
/// store's chunked GET, never buffering the whole file. End-of-stream is the
/// command's promise resolving.
#[tauri::command]
pub(crate) async fn files_download(
    state: State<'_, AppState>,
    path: String,
    offset: Option<i64>,
    length: Option<i64>,
    on_chunk: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    let files = state.snapshot()?.files()?;
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
pub(crate) async fn files_upload(
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
    let files = state.snapshot()?.files()?;
    let meta = files
        .put_file_stream(&path, content_type, chunks)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(meta).map_err(|e| e.to_string())
}

// --- Environment management -----------------------------------------------------

/// List the configured environments. Empty on a fresh install (the shell then
/// shows the create flow).
#[tauri::command]
pub(crate) fn list_environments() -> Result<Vec<Environment>, String> {
    read_environments()
}

/// The currently-active environment descriptor (services bound), or `null` when
/// none is active. The shell uses this to scope its state and highlight the
/// running environment in the overview; `null` at startup lands it on the
/// environment manager (no environment is auto-activated).
#[tauri::command]
pub(crate) fn active_environment(state: State<'_, AppState>) -> Option<serde_json::Value> {
    active_environment_descriptor(&state)
}

/// Create an environment: allocate an id, create its data dir, and append it to
/// the registry. Does NOT spawn any services — selection does that.
#[tauri::command]
pub(crate) fn create_environment(name: String) -> Result<Environment, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("environment name must not be empty".into());
    }
    let mut envs = read_environments()?;
    let id = allocate_env_id(&name, &envs);
    let uc_dir = env_uc_dir(&id);
    std::fs::create_dir_all(&uc_dir).map_err(|e| format!("creating {uc_dir:?}: {e}"))?;
    let home_dir = env_home_dir(&id);
    std::fs::create_dir_all(&home_dir).map_err(|e| format!("creating {home_dir:?}: {e}"))?;

    // Mint a fresh per-environment KEK in the OS keychain (the default provider)
    // so credentials are protected from first use. A keychain failure is NOT
    // fatal to creation — the environment is created in an `Unavailable` key
    // status so the UI can warn and let the user choose a provider before start.
    if let Err(e) = kek::configure(&id, &uc_dir, kek::KeyProvider::Keychain) {
        eprintln!("[kek] key provisioning deferred for {id}: {e}");
    }

    let env = Environment {
        id,
        name,
        modules: Vec::new(),
        observability: false,
    };
    envs.push(env.clone());
    write_environments(&envs)?;
    Ok(env)
}

/// Start an environment: tear down any running sidecar, write its UC config,
/// spawn the `uc` server, seed it, and bind the in-process executors. Returns the
/// active-environment descriptor. Re-starting an environment respawns it cleanly.
/// Starting does not open the app — the UI decides whether to navigate into it.
#[tauri::command]
pub(crate) async fn start_environment(
    app: tauri::AppHandle,
    id: String,
) -> Result<serde_json::Value, String> {
    let envs = read_environments()?;
    let env = envs
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("unknown environment: {id}"))?;
    // The persisted module selection (catalog module ids) and observability opt-in.
    let modules = env.modules.clone();
    let observability = env.observability;

    // Already active → no-op (the UI re-opens an already-running environment
    // without calling start, but keep the command idempotent so a redundant
    // call doesn't pointlessly kill + respawn the sidecar). Return the current
    // descriptor so the caller can scope state either way.
    let already_active = app.state::<AppState>().active.read().unwrap().id.as_deref() == Some(&id);
    if already_active {
        return active_environment_descriptor(&app.state::<AppState>())
            .ok_or_else(|| "environment reported active but has no descriptor".to_string());
    }

    // Refuse to start with an unusable encryption key rather than spawning a
    // sidecar that would fail to decrypt credentials (or silently fall back to a
    // shared key — which we never do). The UI surfaces this as a blocking warning.
    let uc_dir = env_uc_dir(&id);
    if kek::status(&id, &uc_dir) == kek::KeyStatus::Unavailable {
        return Err(
            "no usable encryption key for this environment — the OS keychain is \
             unavailable. Configure a key store before starting."
                .into(),
        );
    }

    // Switching to a different environment: tear down the previously-running
    // environment's processes (UC sidecar, its compose project, and any uvx
    // sidecars like marimo) before spawning this one's. The already-active case
    // returned above. Clear notebook bookkeeping too so the next environment's
    // marimo server is discovered fresh.
    if let Some(supervisor) = app.try_state::<Supervisor>() {
        supervisor.shut_down_all();
    }
    app.state::<Notebooks>().reset();

    let config_path = uc::write_uc_config(&id, &uc_dir)?;
    let endpoint = uc::spawn_uc_sidecar(&app, &id, &config_path).await?;
    eprintln!("[uc] environment {id} listening at {endpoint}");

    // Observability is a per-env opt-in that emits to the SHARED, app-level
    // telemetry collector (not a per-env service). Bring the collector up + init
    // the global tracer lazily on the first opt-in env; later envs reuse it. This
    // lives in the app-level Telemetry slot, not the per-env supervisor, so it
    // survives env switches. Done FIRST (before the compose services) so the
    // collector exists when the services' OTLP exporters point at it, and so the
    // in-process engine emits too. A failure tears UC back down (the user asked
    // for observability and we couldn't provide it).
    if observability {
        if let Err(e) = telemetry::ensure(&app.state::<Telemetry>()) {
            app.state::<Supervisor>().shut_down_all();
            return Err(e);
        }
    }

    // Bring up the environment's capability services (Docker compose project)
    // BEFORE building the in-process engine: an effect like lineage produces an
    // endpoint (the Marquez sink via the gateway) that the engine must be
    // configured with, and that endpoint only exists once the services are
    // healthy. Tracked in the supervisor so they tear down with the environment;
    // a failure tears UC back down rather than leaving it orphaned.
    let mut lineage_endpoint = None;
    if !modules.is_empty() {
        let supervisor = app.state::<Supervisor>();
        match modules::start_modules(&id, &modules, &supervisor) {
            Ok(plan) => lineage_endpoint = modules::lineage_endpoint(&plan),
            Err(e) => {
                supervisor.shut_down_all();
                return Err(e);
            }
        }
    }

    // Now build the in-process engine, wired with any effect-derived endpoints
    // (lineage). The engine consumes the effects; it is built exactly once.
    crate::env::activate_endpoint(
        &app,
        Some(id.clone()),
        Some(endpoint.clone()),
        lineage_endpoint,
    )
    .await?;

    active_environment_descriptor(&app.state::<AppState>())
        .ok_or_else(|| "activation succeeded but produced no descriptor".to_string())
}

/// Stop an environment: kill its UC sidecar and clear the active services so the
/// shell returns to the idle/overview state. Idempotent — a no-op when the given
/// id is not the active one (or nothing is active). Only the single active
/// environment can be running today, so stopping any other id does nothing.
#[tauri::command]
pub(crate) async fn stop_environment(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();

    // Ignore stop for an environment that is not the active one.
    {
        let active = state.active.read().unwrap();
        if active.id.as_deref() != Some(&id) {
            return Ok(());
        }
    }

    // Tear down every process bound to this environment (UC sidecar, and later
    // the compose project / uvx sidecars). Draining the supervisor leaves the
    // exit hook nothing to do — no double-kill.
    if let Some(supervisor) = app.try_state::<Supervisor>() {
        supervisor.shut_down_all();
    }
    app.state::<Notebooks>().reset();

    // Reset the active services to "none selected". `snapshot()` then errors and
    // the proxy reports "Unity Catalog is not running" until the next start.
    *state.active.write().unwrap() = crate::env::ActiveEnv::default();
    Ok(())
}

/// Current encryption-key status for an environment (without starting it). Drives
/// the key-management surface in the environment overview.
#[tauri::command]
pub(crate) fn environment_key_status(id: String) -> Result<kek::KeyStatus, String> {
    let envs = read_environments()?;
    if !envs.iter().any(|e| e.id == id) {
        return Err(format!("unknown environment: {id}"));
    }
    Ok(kek::status(&id, &env_uc_dir(&id)))
}

/// Configure the encryption-key provider for an environment, returning the
/// resulting status. For the keychain provider this mints the key eagerly so a
/// broken keychain surfaces here rather than at start.
///
/// The key material is minted once and never rotated, so the provider is **locked**
/// after a key exists: changing it would orphan already-sealed credentials. We
/// reject a *change* of provider once a key record is present, but allow a no-op
/// re-configure to the same provider (e.g. retrying after a transient keychain
/// failure left the env unconfigured).
#[tauri::command]
pub(crate) fn configure_environment_key(
    id: String,
    provider: kek::KeyProvider,
) -> Result<kek::KeyStatus, String> {
    let envs = read_environments()?;
    if !envs.iter().any(|e| e.id == id) {
        return Err(format!("unknown environment: {id}"));
    }
    let uc_dir = env_uc_dir(&id);
    if let Some(existing) = kek::read_key_config(&uc_dir) {
        if existing.provider != provider {
            return Err(format!(
                "this environment's key is already provisioned with the {:?} provider; \
                 the provider can't be changed without rotation (not supported)",
                existing.provider
            ));
        }
    }
    kek::configure(&id, &uc_dir, provider)
}

/// Turn Touch ID protection on or off for an environment's keychain-stored key.
/// Rewrites the same key material with/without the biometric access-control flag —
/// no rotation — and returns the resulting status. macOS only; elsewhere this
/// errors (the UI hides the switch on unsupported hosts).
#[tauri::command]
pub(crate) fn set_environment_key_biometric(
    id: String,
    enabled: bool,
) -> Result<kek::KeyStatus, String> {
    let envs = read_environments()?;
    if !envs.iter().any(|e| e.id == id) {
        return Err(format!("unknown environment: {id}"));
    }
    kek::set_biometric(&id, &env_uc_dir(&id), enabled)
}

/// Whether the Docker daemon is reachable. Drives the UI availability banner
/// (Docker-backed modules are disabled, with install hints, when this is false)
/// and is re-checked at start before bringing a Docker-backed environment up.
#[tauri::command]
pub(crate) fn docker_status() -> bool {
    modules::docker_available()
}

/// Set an environment's selected modules, persisting the registry. Takes effect on
/// the next start (a running environment is not hot-reconfigured). Unknown module
/// ids are rejected so the UI can't persist a module the backend won't resolve.
#[tauri::command]
pub(crate) fn set_environment_modules(id: String, modules: Vec<String>) -> Result<(), String> {
    for module in &modules {
        if !topology::is_known_module(module) {
            return Err(format!("unknown module: {module}"));
        }
    }
    let mut envs = read_environments()?;
    let env = envs
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("unknown environment: {id}"))?;
    env.modules = modules;
    write_environments(&envs)
}

/// Set an environment's observability opt-in, persisting the registry. Takes effect
/// on the next start (a running environment is not hot-reconfigured).
#[tauri::command]
pub(crate) fn set_environment_observability(id: String, enabled: bool) -> Result<(), String> {
    let mut envs = read_environments()?;
    let env = envs
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("unknown environment: {id}"))?;
    env.observability = enabled;
    write_environments(&envs)
}

/// The available modules (id + label) for the UI to render as a checklist.
#[tauri::command]
pub(crate) fn available_modules() -> Vec<serde_json::Value> {
    topology::available_modules()
        .iter()
        .map(|m| serde_json::json!({ "id": m.id, "label": m.label }))
        .collect()
}

/// An environment's currently-selected module ids (for pre-checking the checklist).
/// Empty for a fresh or UC-only environment.
#[tauri::command]
pub(crate) fn environment_modules(id: String) -> Result<Vec<String>, String> {
    let envs = read_environments()?;
    envs.into_iter()
        .find(|e| e.id == id)
        .map(|e| e.modules)
        .ok_or_else(|| format!("unknown environment: {id}"))
}

/// Whether an environment opts in to the shared telemetry collector (for the
/// Observability toggle).
#[tauri::command]
pub(crate) fn environment_observability(id: String) -> Result<bool, String> {
    let envs = read_environments()?;
    envs.into_iter()
        .find(|e| e.id == id)
        .map(|e| e.observability)
        .ok_or_else(|| format!("unknown environment: {id}"))
}

/// Live per-service status (state + health) for a running environment, for the
/// UI's Services panel. Best-effort: returns an empty list when Docker is
/// unavailable or nothing is up. The UI polls this on a gentle interval.
#[tauri::command]
pub(crate) fn environment_service_status(id: String) -> Vec<modules::ServiceStatus> {
    modules::service_status(&id)
}

/// Whether the shared, app-level telemetry collector (Jaeger) is running. Drives
/// the Telemetry entry's status + whether its embedded UI is available.
#[tauri::command]
pub(crate) fn telemetry_status() -> bool {
    modules::telemetry_running()
}

/// The read-only config artifacts (generated compose + the static fragments and
/// gateway/collector configs) for an environment's selected modules, for the
/// teaching/inspection viewer. The compose is generated on demand, so this works
/// before the environment has ever been started.
#[tauri::command]
pub(crate) fn environment_config_artifacts(
    id: String,
) -> Result<Vec<modules::ConfigArtifact>, String> {
    let envs = read_environments()?;
    let env = envs
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("unknown environment: {id}"))?;
    modules::config_artifacts(&env.modules)
}
