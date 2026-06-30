//! Unity Catalog sidecar lifecycle and the UI REST proxy.
//!
//! Unlike the portal/hydrofoil executors (which run in-process via
//! [`desktop_host`]), Unity Catalog must be a real running server. Each
//! environment spawns its own `uc` server as a Tauri sidecar on a dynamic port
//! (see [`spawn_uc_sidecar`]); [`proxy_request`] forwards the webview's UC REST
//! calls to that sidecar.

use tauri::{Manager, State};

use crate::env::AppState;
use crate::kek;
use crate::supervisor::{ManagedProcess, Supervisor};

/// Write the UC server config (SQLite + local `file://` managed storage, all
/// under the given `data_dir`) and return its path. We supply the config explicitly
/// because a provided config file with no `encryption` block deserializes to
/// `None` (the dev-KEK default only applies to a config-LESS launch), which the
/// server rejects — so we include an `encryption` block.
///
/// The KEK is **not** written inline. Instead the config references it via UC's
/// `key: { env: OPEN_LAKEHOUSE_UC_KEK }` indirection; the per-environment key
/// material lives in the OS keychain (see [`kek`]) and is injected into the
/// sidecar process env at spawn. `encryption.active.id` is the stable key id from
/// `key.json` (defaults to the env id), recorded in every sealed secret.
///
/// `managed_storage_root` is `file://<.uc-data/storage>` so catalog data persists
/// on disk (inspectable). A managed catalog requires a resolvable storage root,
/// and the local `file://` root must be covered by `local_storage.allowed-roots`
/// (deny-by-default governance). NOTE the key casing — this is the easy trap:
/// `Config` fields are snake_case (`local_storage`, `managed_storage_root`) but
/// the nested `LocalStorageConfig` is kebab-case (`allowed-roots`). Using the
/// wrong case silently drops the allow-root → "local storage is not enabled".
pub(crate) fn write_uc_config(
    env_id: &str,
    data_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("creating {data_dir:?}: {e}"))?;
    let canonical = std::fs::canonicalize(data_dir).map_err(|e| e.to_string())?;
    let db_path = canonical.join("catalog.db");
    let storage_root = canonical.join("storage");
    std::fs::create_dir_all(&storage_root).map_err(|e| e.to_string())?;

    // Stable KEK id stamped into every sealed secret. From `key.json` when present
    // (set at create/configure time), else the env id.
    let key_id = kek::read_key_config(&canonical)
        .map(|c| c.key_id)
        .unwrap_or_else(|| env_id.to_string());

    let config = format!(
        "host: 127.0.0.1\n\
         port: 0\n\
         backend:\n\
         \x20\x20engine: sqlite\n\
         \x20\x20path: {db}\n\
         encryption:\n\
         \x20\x20active:\n\
         \x20\x20\x20\x20id: {key_id}\n\
         \x20\x20\x20\x20key:\n\
         \x20\x20\x20\x20\x20\x20env: {kek_env}\n\
         local_storage:\n\
         \x20\x20allowed-roots:\n\
         \x20\x20\x20\x20- {root}\n\
         managed_storage_root: \"file://{root}\"\n",
        db = db_path.display(),
        kek_env = kek::KEK_ENV_VAR,
        root = storage_root.display(),
    );
    let config_path = canonical.join("config.yaml");
    std::fs::write(&config_path, config).map_err(|e| format!("writing config: {e}"))?;
    Ok(config_path)
}

/// Spawn the local `uc` server (SQLite, port 0), scrape the bound port from its
/// startup line, and return the REST endpoint. Stores the child in managed state
/// so the exit hook can kill it.
pub(crate) async fn spawn_uc_sidecar(
    app: &tauri::AppHandle,
    env_id: &str,
    config_path: &std::path::Path,
) -> Result<String, String> {
    use tauri_plugin_shell::process::CommandEvent;
    use tauri_plugin_shell::ShellExt;

    // Resolve the per-environment KEK from the OS keychain (get-or-create) and
    // hand it to the child via env only — the config references it as
    // `key: { env: OPEN_LAKEHOUSE_UC_KEK }`, so the material never hits disk.
    // `key.json` (the biometric flag for new keys) sits beside config.yaml.
    let uc_dir = config_path.parent().unwrap_or(config_path);
    let kek = kek::ensure_kek(env_id, uc_dir)?;

    // Spawn as a Tauri sidecar: the binary lives at
    // `src-tauri/binaries/uc-server-<target-triple>` (declared in tauri.conf.json
    // `externalBin`; `just uc-setup` symlinks the sibling build there). Tauri
    // resolves the triple-suffixed path itself, so we avoid the shell scope's
    // `cmd`-string path resolution entirely.
    let (mut rx, child) = app
        .shell()
        .sidecar("uc-server")
        .map_err(|e| format!("uc-server sidecar not found (run `just uc-setup`?): {e}"))?
        .env(kek::KEK_ENV_VAR, kek)
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

    // Track the child in the supervisor so the exit hook (and `stop_environment`)
    // can kill it alongside any compose project / uvx sidecars. The prior
    // environment's processes are torn down by `start_environment` before this
    // spawn, so the supervisor holds only the current environment's set.
    if let Some(supervisor) = app.try_state::<Supervisor>() {
        supervisor.track(ManagedProcess::Sidecar {
            label: "uc-server".to_string(),
            child,
        });
    }

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

/// JSON response shape for `proxy_request` (matches `ProxyResponse` in
/// tauri-fetch.ts).
#[derive(serde::Serialize)]
pub(crate) struct ProxyResponse {
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
pub(crate) async fn proxy_request(
    state: State<'_, AppState>,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
) -> Result<ProxyResponse, String> {
    let endpoint = state
        .unity_endpoint()
        .ok_or("Unity Catalog is not running")?;
    let endpoint = endpoint.as_str();

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
    let resp_body = resp
        .text()
        .await
        .map_err(|e| format!("uc proxy body: {e}"))?;
    Ok(ProxyResponse {
        status,
        body: resp_body,
        headers: resp_headers,
    })
}
