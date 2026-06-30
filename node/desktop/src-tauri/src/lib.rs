//! Open Lakehouse desktop shell (Tauri v2).
//!
//! The desktop app runs the portal (Tags + Files) and hydrofoil (QueryService +
//! IngestService) executors **in-process** instead of over HTTP, so a local run
//! needs no Docker Compose stack for those services. Only Unity Catalog must be a
//! real server, reached over HTTP — spawned per environment as a Tauri sidecar on
//! a dynamic port (see [`uc::spawn_uc_sidecar`]). Heavier services (Lineage, MLflow)
//! stay in Compose.
//!
//! Module map:
//!   - [`paths`] — app data-dir layout shared across modules.
//!   - [`env`] — the environments registry model, persistence, and the
//!     active-environment state commands snapshot against.
//!   - [`uc`] — the Unity Catalog sidecar lifecycle + the UI REST proxy.
//!   - [`commands`] — the Tauri command surface the webview invokes (the in-process
//!     `connect_*` / `files_*` dispatchers plus environment management).
//!   - [`modules`] / [`topology`] / [`telemetry`] / [`supervisor`] — Docker-compose
//!     module orchestration, the shared topology catalog, the app-level telemetry
//!     collector, and the managed-process registry.
//!   - [`kek`] — per-environment key-encryption-key management (OS keychain).
//!   - [`notebook`] — the marimo notebook sidecar + working copies.
//!
//! The UI reaches the executors through Tauri commands (see `tauri-transport.ts`
//! / the Files host seam on the JS side):
//!   - `connect_unary` / `connect_unary_proto` / `connect_stream` / `query_ingest`
//!     drive the `connectrpc::Router` dispatchers for **Tags**, the
//!     **QueryService**, and the **IngestService** — JSON in/out (unary), proto
//!     in/out (unary, for the IPC-carrying `PreviewFile`), raw Connect frames over
//!     a `Channel` (server-streaming `RunQuery`), or a list of proto frames
//!     (client-streaming `IngestTable`).
//!   - `files_*` call the `FileStore` directly with native types (no proto
//!     framing) — the store already is the sanitized handler.

mod commands;
mod env;
mod kek;
mod modules;
mod notebook;
mod paths;
mod supervisor;
mod telemetry;
mod topology;
mod uc;

// Re-exports for the sibling modules that still address these by crate path
// (`crate::app_data_dir`, `crate::AppState`).
pub(crate) use env::AppState;
pub(crate) use paths::app_data_dir;

use tauri::Manager;

use notebook::Notebooks;
use supervisor::Supervisor;
use telemetry::Telemetry;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        // Tracks every process bound to the active environment (UC sidecar, and
        // later the compose project / uvx sidecars) so the exit hook and
        // `stop_environment` can tear the whole set down. Empty until first spawn.
        .manage(Supervisor::default())
        // App-level shared telemetry (one Jaeger for all environments). Started
        // lazily by the first observability-enabled environment; lives for the
        // app's lifetime (survives env switches), torn down only on app exit.
        .manage(Telemetry::default())
        // Per-environment marimo notebook server + working copies. Empty until
        // the first `.py` notebook opens; reset on environment teardown (the
        // marimo child itself is killed via the Supervisor). The notebook tab
        // embeds the marimo UI by pointing its iframe directly at the sidecar's
        // loopback URL (so marimo's WebSocket connects natively) — see
        // `notebook::open_notebook`.
        .manage(Notebooks::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_environments,
            commands::active_environment,
            commands::create_environment,
            commands::start_environment,
            commands::stop_environment,
            commands::environment_key_status,
            commands::configure_environment_key,
            commands::set_environment_key_biometric,
            commands::docker_status,
            commands::set_environment_modules,
            commands::set_environment_observability,
            commands::available_modules,
            commands::environment_modules,
            commands::environment_observability,
            commands::environment_config_artifacts,
            commands::environment_service_status,
            commands::telemetry_status,
            commands::connect_unary,
            commands::connect_unary_proto,
            commands::connect_stream,
            commands::query_ingest,
            commands::files_stat,
            commands::files_list,
            commands::files_create_dir,
            commands::files_delete,
            commands::files_delete_dir,
            commands::files_download,
            commands::files_upload,
            uc::proxy_request,
            notebook::open_notebook,
            notebook::sync_notebook,
            notebook::close_notebook,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Tear down every process bound to the active environment when the
            // app exits (UC server, and later the compose project / uvx sidecars),
            // plus the shared telemetry collector (app-lifetime, so only here).
            if let tauri::RunEvent::Exit = event {
                if let Some(supervisor) = app.try_state::<Supervisor>() {
                    supervisor.shut_down_all();
                }
                if let Some(telemetry) = app.try_state::<Telemetry>() {
                    telemetry.shut_down();
                }
            }
        });
}
