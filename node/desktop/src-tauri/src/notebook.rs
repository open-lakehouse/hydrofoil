//! Marimo notebook sidecar + per-file working copies.
//!
//! Opening a `.py` file in the editor as a notebook (desktop only) copies it
//! into a sandboxed working directory, ensures a single shared `marimo edit`
//! server is running for the active environment, and hands the UI the marimo
//! server's loopback URL (`http://127.0.0.1:<port>/?file=<rel>`) to embed in an
//! iframe. The iframe points DIRECTLY at the loopback server (not a proxy): the
//! marimo frontend derives its WebSocket URL from `document.baseURI`, so a
//! proxied/custom-scheme document would yield an unreachable WS — the loopback
//! URL lets its reactive runtime connect natively. marimo sends no
//! `X-Frame-Options` / CSP, so loopback framing is allowed.
//!
//! Topology mirrors the UC sidecar (`spawn_uc_sidecar` in `lib.rs`): one
//! `uvx`-spawned child, its port scraped from stdout, tracked in the
//! [`Supervisor`](crate::supervisor) so `stop_environment` and the app-exit
//! hook tear it down. The server is **per-environment, not per-tab** — every
//! open notebook is served by the same marimo process from one working dir, so
//! no per-tab process lifecycle is needed.
//!
//! Python environment: marimo is run via `uvx` with the Unity Catalog client
//! (`unitycatalog-client[obstore]` + `obstore`) injected as `--with` deps, and
//! **not** `--sandbox`. `uvx` already gives the marimo *server* its own
//! uv-managed environment; since there is one server per OL environment, all
//! notebook tabs share that one Python env. `--sandbox` would add a *second*
//! isolation layer (a per-notebook venv from each notebook's PEP 723 deps),
//! which we don't want: it's the user's own environment, and it would re-resolve
//! the non-PyPI UC client wheel per notebook. With obstore + the UC client in the
//! shared env, a notebook cell can build a UC-vended `obstore` store
//! (`unitycatalog_client.obstore.store_for_volume`) that marimo's Files panel
//! auto-discovers as a browsable remote source. The UC client is a compiled PyO3
//! wheel that is not on PyPI (and whose name collides with an unrelated PyPI
//! package), so it is injected by **direct wheel path** (see [`uc_client_wheel`]);
//! when the wheel is absent the obstore engines fail to import but the editor
//! still works.
//!
//! Data access: the child inherits `OPEN_LAKEHOUSE_UC_URL` / `UC_URI` pointing
//! at the host UC sidecar, so notebook cells can reach Unity Catalog (the same
//! one-way host→child injection idea as `UC_HOST_URL` for compose). When the
//! environment carries the lineage capability it also inherits `LINEAGE_URL`
//! (the Envoy gateway base) so notebook templates can wire OpenLineage. No UC
//! token is forwarded yet — the desktop UC sidecar is unauthenticated today.
//!
//! Config: a per-environment `.marimo.toml` is generated into the workdir (see
//! [`write_marimo_config`]). marimo discovers user config by searching from its
//! working directory upward for a `.marimo.toml`; since the child's working
//! directory is the workdir, the file we drop there is the closest match and
//! wins. It turns off marimo's AI features (no phone-home until an in-network
//! gateway exists), enables autosave + format-on-save, and sets
//! `follow_symlink = false` so the file browser can't escape the workdir via
//! symlinks. (marimo OSS does not bound the explorer endpoints to a hard root,
//! so a user typing an explicit parent path can still navigate up; that is
//! acceptable for this local, single-user, loopback-only desktop.)

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::Manager;

use crate::supervisor::{ManagedProcess, Supervisor};

/// One prepared notebook: the volume path it was opened from and the working
/// copy on disk that marimo edits. The working copy is synced back to the
/// volume on save/close and discarded when the session is released.
struct Session {
    /// The volume path the notebook was opened from (where edits sync back to).
    volume_path: String,
    /// The working copy under `.open-lakehouse/envs/<id>/notebooks/`.
    working_path: PathBuf,
}

/// Managed state for the active environment's notebook server + sessions.
/// Reset (sessions cleared, port dropped) when the environment is torn down;
/// the marimo child itself is killed via the supervisor.
#[derive(Default)]
pub struct Notebooks {
    inner: Mutex<NotebooksInner>,
}

#[derive(Default)]
struct NotebooksInner {
    /// The shared marimo server's loopback base, e.g. `http://127.0.0.1:PORT`.
    /// `None` until the first notebook opens; reused for every notebook in the
    /// environment.
    endpoint: Option<String>,
    /// Open sessions, keyed by an opaque session id handed to the UI.
    sessions: HashMap<String, Session>,
}

impl Notebooks {
    /// Clear all notebook state on environment teardown. The marimo child is
    /// killed separately via the supervisor; this just drops our bookkeeping so
    /// the next environment starts fresh.
    pub fn reset(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.endpoint = None;
        inner.sessions.clear();
    }
}

/// A stable, filesystem-safe working-copy name for a volume path: its hex-encoded
/// FNV-1a hash plus the original extension, so two different volume paths never
/// collide and re-opening the same path reuses one working copy.
fn working_name(volume_path: &str) -> String {
    // FNV-1a (64-bit) — small, dependency-free, sufficient for path keying.
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in volume_path.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let ext = volume_path
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(_, e)| e))
        .filter(|e| !e.is_empty())
        .unwrap_or("py");
    format!("{hash:016x}.{ext}")
}

/// The per-environment marimo user config. Written as `.marimo.toml` into the
/// workdir, which is the child's working directory; marimo's upward search from
/// cwd finds it first, so it applies to this environment's notebooks only.
///
/// Goals: kill UI/UX noise (no AI phone-home until an in-network gateway exists),
/// improve the editing experience (autosave + format-on-save, `ty` language
/// server), match the app chrome (dark theme), and keep the file browser from
/// escaping the workdir via symlinks (`follow_symlink = false`).
const MARIMO_CONFIG: &str = "\
# Generated per-environment by open-lakehouse; do not edit by hand.
[package_management]
manager = \"uv\"

[ai]
enabled = false

[save]
autosave = \"after_delay\"
autosave_delay = 1000
format_on_save = true

[formatting]
line_length = 88

[language_servers.ty]
enabled = true

[display]
theme = \"dark\"
default_width = \"medium\"

[server]
follow_symlink = false
";

/// Write the per-environment `.marimo.toml` into `workdir`. Idempotent
/// (overwrites): the config is static, so rewriting on each ensure is cheap and
/// keeps the file in sync if its contents change between releases.
fn write_marimo_config(workdir: &std::path::Path) -> Result<(), String> {
    let path = workdir.join(".marimo.toml");
    std::fs::write(&path, MARIMO_CONFIG).map_err(|e| format!("writing marimo config: {e}"))
}

/// The local `unitycatalog-client` wheel passed to `uvx --with` so notebooks can
/// import the (non-PyPI) Unity Catalog client. We pass the wheel by **direct
/// path** (`unitycatalog_client[obstore] @ /abs/path.whl`) rather than by name +
/// `UV_FIND_LINKS`, because PyPI hosts an *unrelated* package also named
/// `unitycatalog-client` (the official client, a higher version) that would
/// otherwise win resolution. A direct path is unambiguous and needs no version
/// pin.
///
/// The wheel directory is `OPEN_LAKEHOUSE_UC_WHEELS` if set, else
/// `node/desktop/wheels/` in a dev checkout (built by `just node build-uc-wheel`).
/// Returns the first `unitycatalog_client-*.whl` found, or `None` when absent —
/// the spawn then omits the `--with`, so the obstore engines fail to import but
/// the editor still works.
fn uc_client_wheel() -> Option<PathBuf> {
    let dir = match std::env::var("OPEN_LAKEHOUSE_UC_WHEELS") {
        Ok(d) => PathBuf::from(d),
        // Dev fallback: the desktop crate's manifest dir is
        // `node/desktop/src-tauri`, so the wheels live one level up.
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()?
            .join("wheels"),
    };
    let entries = std::fs::read_dir(&dir).ok()?;
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("unitycatalog_client-") && n.ends_with(".whl"))
        })
}

/// Ensure the shared marimo server is running for the active environment and
/// return its loopback base (`http://127.0.0.1:PORT`). Idempotent: the first
/// call spawns the child (tracked in the supervisor) and scrapes its port;
/// later calls return the cached endpoint.
///
/// `uc_url` is the host UC REST base injected into the child so notebook cells
/// can reach Unity Catalog; `None` when UC is disabled. `lineage_url` is the
/// OpenLineage sink base injected as `LINEAGE_URL`; `None` when the environment
/// has no lineage capability.
async fn ensure_marimo(
    app: &tauri::AppHandle,
    workdir: &std::path::Path,
    uc_url: Option<&str>,
    lineage_url: Option<&str>,
) -> Result<String, String> {
    use tauri_plugin_shell::ShellExt;
    use tauri_plugin_shell::process::CommandEvent;

    // Fast path: already running for this environment.
    {
        let notebooks = app.state::<Notebooks>();
        let inner = notebooks.inner.lock().unwrap();
        if let Some(endpoint) = &inner.endpoint {
            return Ok(endpoint.clone());
        }
    }

    std::fs::create_dir_all(workdir).map_err(|e| format!("creating notebook workdir: {e}"))?;
    write_marimo_config(workdir)?;

    // `marimo edit --headless` serves every notebook under `workdir`. `-p 0`
    // asks for a free port (marimo falls back to its default 2718 and increments
    // if busy — we scrape the actual port from stdout regardless). `--no-token`
    // is acceptable because the server is loopback-only. Run via `uvx`, injecting
    // the Unity Catalog client (`unitycatalog-client[obstore]` + `obstore`) into
    // the marimo server's shared environment so notebook cells can build a
    // UC-vended obstore store. `--no-sandbox`: `uvx` already isolates the server's
    // env, and we don't want a second per-notebook venv (see module docs). The
    // child's working directory is the workdir so marimo's file explorer
    // (`os.getcwd()`) defaults there.
    let mut uvx_args: Vec<String> = Vec::new();
    // Inject the local UC client wheel by direct path (see `uc_client_wheel`),
    // plus obstore from PyPI. When the wheel is absent, the obstore engines fail
    // to import but the editor still works.
    if let Some(wheel) = uc_client_wheel() {
        uvx_args.push("--with".into());
        uvx_args.push(format!(
            "unitycatalog_client[obstore] @ {}",
            wheel.to_string_lossy()
        ));
        uvx_args.push("--with".into());
        uvx_args.push("obstore".into());
    }
    uvx_args.extend(
        [
            "marimo",
            "edit",
            "--headless",
            "--host",
            "127.0.0.1",
            "-p",
            "0",
            "--no-token",
            "--no-sandbox",
            &workdir.to_string_lossy(),
        ]
        .into_iter()
        .map(String::from),
    );
    let mut cmd = app.shell().command("uvx").current_dir(workdir).args(uvx_args);
    if let Some(url) = uc_url {
        // Inject under both names notebooks look for (see notebooks/_caspers_read
        // / _demo_auth conventions); host→child, one direction only.
        cmd = cmd.env("OPEN_LAKEHOUSE_UC_URL", url).env("UC_URI", url);
    }
    if let Some(url) = lineage_url {
        // Notebook templates append `/api/v1/lineage` to this base (matching
        // notebooks/spark_lineage.py); host→child, one direction only.
        cmd = cmd.env("LINEAGE_URL", url);
    }

    let (mut rx, child) = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn marimo (is `uvx` on PATH?): {e}"))?;

    if let Some(supervisor) = app.try_state::<Supervisor>() {
        supervisor.track(ManagedProcess::Sidecar {
            label: "marimo".to_string(),
            child,
        });
    }

    // marimo prints its URL on startup, e.g. `URL: http://127.0.0.1:PORT`.
    // Scrape the loopback base from the first line that carries one.
    while let Some(event) = rx.recv().await {
        let line = match event {
            CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                String::from_utf8_lossy(&bytes).into_owned()
            }
            CommandEvent::Terminated(payload) => {
                return Err(format!(
                    "marimo exited before announcing its address (code {:?})",
                    payload.code
                ));
            }
            _ => continue,
        };
        if let Some(base) = parse_marimo_base(&line) {
            let notebooks = app.state::<Notebooks>();
            notebooks.inner.lock().unwrap().endpoint = Some(base.clone());
            return Ok(base);
        }
    }
    Err("marimo stream ended before announcing its address".into())
}

/// Extract the loopback base (`http://localhost:PORT` / `http://127.0.0.1:PORT`)
/// from a marimo startup line (`➜  URL: http://localhost:PORT`). Tolerates the
/// emoji/text prefix by scanning for the `http://` token, then takes the
/// authority (`host:port`) up to the first `/` or whitespace.
fn parse_marimo_base(line: &str) -> Option<String> {
    let start = line.find("http://")?;
    let after_scheme = &line[start + "http://".len()..];
    let authority_len = after_scheme
        .find(|c: char| c == '/' || c.is_whitespace())
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_len];
    if authority.contains(':') {
        Some(format!("http://{authority}"))
    } else {
        None
    }
}

/// The notebook working dir for an environment:
/// `.open-lakehouse/envs/<id>/notebooks`.
fn env_notebooks_dir(id: &str) -> PathBuf {
    crate::app_data_dir()
        .join("envs")
        .join(id)
        .join("notebooks")
}

/// Response for `open_notebook`: the iframe URL + the session handle.
#[derive(serde::Serialize)]
pub struct OpenedNotebook {
    url: String,
    session_id: String,
}

/// Open a volume `.py` file as a marimo notebook: copy it into the env's
/// notebook workdir, ensure the shared marimo server is up, and return the
/// marimo loopback URL (`http://127.0.0.1:<port>/?file=<rel>`) the UI embeds,
/// plus a session id for later sync/close.
#[tauri::command]
pub async fn open_notebook(
    app: tauri::AppHandle,
    path: String,
) -> Result<OpenedNotebook, String> {
    let state = app.state::<crate::AppState>();
    let (env_id, files, uc_url, lineage_url) = {
        let active = state.active.read().unwrap();
        let id = active
            .id
            .clone()
            .ok_or("no environment selected")?;
        let files = active
            .hosted
            .as_ref()
            .ok_or("no environment selected")?
            .files
            .clone();
        (
            id,
            files,
            active.unity_endpoint.clone(),
            active.lineage_endpoint.clone(),
        )
    };

    let workdir = env_notebooks_dir(&env_id);
    std::fs::create_dir_all(&workdir).map_err(|e| format!("creating notebook workdir: {e}"))?;

    // Read the source from the volume via the in-process FileStore (so UC
    // volumes work, not just local Home) and write the working copy.
    let bytes = files
        .read_file(&path, None, None)
        .await
        .map_err(|e| format!("reading notebook {path}: {e}"))?;
    let working_path = workdir.join(working_name(&path));
    std::fs::write(&working_path, &bytes).map_err(|e| format!("writing working copy: {e}"))?;

    let base = ensure_marimo(&app, &workdir, uc_url.as_deref(), lineage_url.as_deref()).await?;

    // Record the session and remember the workdir for the proxy's sake.
    let session_id = working_name(&path); // stable per volume path → reopen reuses it
    {
        let notebooks = app.state::<Notebooks>();
        let mut inner = notebooks.inner.lock().unwrap();
        inner.sessions.insert(
            session_id.clone(),
            Session {
                volume_path: path.clone(),
                working_path: working_path.clone(),
            },
        );
    }

    // marimo opens a file via `?file=<path-relative-to-workdir>`; the working
    // copy's name is that relative path.
    let file_rel = working_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(session_id.clone());

    // Point the iframe DIRECTLY at the loopback marimo server rather than the
    // `olservice://` proxy. marimo's frontend derives its WebSocket URL from
    // `document.baseURI` (it rewrites the page scheme http→ws / https→wss); a
    // custom-scheme document would yield an unreachable `ws://notebook/...`,
    // breaking the reactive runtime. With the iframe pointed at the loopback
    // URL, `document.baseURI` is `http://127.0.0.1:PORT/`, so marimo connects
    // its WS straight to the server. This works because marimo sends no
    // `X-Frame-Options` / CSP `frame-ancestors`, so loopback framing is allowed
    // (verified against a live server). Trade-off: the iframe is cross-origin to
    // the webview, so the cosmetic `customizeFrame` DOM tweaks don't apply — but
    // we open straight to a file, not marimo's home page, so they're moot.
    Ok(OpenedNotebook {
        url: format!("{}/?file={file_rel}", base.trim_end_matches('/')),
        session_id,
    })
}

/// Flush a notebook's working copy back to its volume path (autosave / pre-close).
#[tauri::command]
pub async fn sync_notebook(app: tauri::AppHandle, session_id: String) -> Result<(), String> {
    let (volume_path, working_path, files) = {
        let state = app.state::<crate::AppState>();
        let active = state.active.read().unwrap();
        let files = active
            .hosted
            .as_ref()
            .ok_or("no environment selected")?
            .files
            .clone();
        let notebooks = app.state::<Notebooks>();
        let inner = notebooks.inner.lock().unwrap();
        let session = inner
            .sessions
            .get(&session_id)
            .ok_or("unknown notebook session")?;
        (session.volume_path.clone(), session.working_path.clone(), files)
    };

    let bytes = std::fs::read(&working_path)
        .map_err(|e| format!("reading working copy for sync: {e}"))?;
    files
        .put_file(&volume_path, Some("text/x-python".to_string()), bytes)
        .await
        .map_err(|e| format!("writing notebook back to {volume_path}: {e}"))?;
    Ok(())
}

/// Release a notebook session: drop its working copy and bookkeeping. The
/// shared marimo server keeps running for other notebooks (it is torn down with
/// the environment, not per-tab).
#[tauri::command]
pub async fn close_notebook(app: tauri::AppHandle, session_id: String) -> Result<(), String> {
    let notebooks = app.state::<Notebooks>();
    let working_path = {
        let mut inner = notebooks.inner.lock().unwrap();
        inner.sessions.remove(&session_id).map(|s| s.working_path)
    };
    if let Some(path) = working_path {
        // Best-effort: a missing file is fine.
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_marimo_base() {
        assert_eq!(
            parse_marimo_base("        URL: http://127.0.0.1:2718"),
            Some("http://127.0.0.1:2718".to_string())
        );
        assert_eq!(
            parse_marimo_base("http://127.0.0.1:55012/?token=x"),
            Some("http://127.0.0.1:55012".to_string())
        );
        assert_eq!(parse_marimo_base("no url here"), None);
    }

    #[test]
    fn working_name_is_stable_and_keeps_ext() {
        let a = working_name("/home/foo.py");
        assert!(a.ends_with(".py"));
        assert_eq!(a, working_name("/home/foo.py"));
        assert_ne!(a, working_name("/home/bar.py"));
    }
}
