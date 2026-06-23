//! App-level shared telemetry: one Jaeger collector for ALL environments.
//!
//! Observability is a per-environment *opt-in to emit*, but the sink is a single,
//! shared, app-scoped service — telemetry is interesting across environments and
//! OpenTelemetry initializes once per process. So:
//!
//! - The Jaeger collector is started **lazily** the first time an
//!   observability-enabled environment starts, and lives for the app's lifetime
//!   (a separate slot from the per-environment [`Supervisor`]). It is NOT torn
//!   down on environment switch — only on app exit.
//! - The global OpenTelemetry tracer provider is initialized **once**, on that
//!   same first opt-in, pointing at the shared collector. Environments that don't
//!   opt in simply don't build their engine with tracing enabled, so they emit
//!   nothing even though the provider exists.
//!
//! This module owns only the shared infrastructure (collector + global init). Per
//! environment, whether the engine emits is decided in `lib.rs`/`desktop-host`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use desktop_host::telemetry::OtelGuard;

use crate::supervisor::compose_down;

/// The fixed, app-scoped Compose project name for the shared collector. Distinct
/// from any environment's `ol-<id>` project so it survives env switches.
pub(crate) const TELEMETRY_PROJECT: &str = "ol-telemetry";

/// App-level shared telemetry state, managed separately from per-environment
/// processes. Holds the global OTLP guard (kept alive for the app's lifetime;
/// dropped on exit to flush spans) once telemetry is initialized.
#[derive(Default)]
pub struct Telemetry {
    inner: Mutex<Option<OtelGuard>>,
}

impl Telemetry {
    /// Tear down the shared collector and flush the tracer (app exit only).
    pub fn shut_down(&self) {
        // Dropping the guard flushes + shuts down the tracer provider.
        let _ = self.inner.lock().unwrap().take();
        compose_down(TELEMETRY_PROJECT);
    }
}

/// Ensure the shared telemetry collector is running and the global tracer is
/// initialized — idempotent and called only when an observability-enabled
/// environment starts. The first call starts Jaeger and initializes OTLP; later
/// calls are no-ops. Errors are surfaced so a broken telemetry start doesn't
/// silently disable observability.
pub fn ensure(telemetry: &Telemetry) -> Result<(), String> {
    let mut guard = telemetry.inner.lock().unwrap();
    if guard.is_some() {
        return Ok(()); // already up
    }

    // Bring up the shared Jaeger (its own app-scoped project; reconcile any stale
    // one from a prior crash first, mirroring the per-env path).
    compose_down(TELEMETRY_PROJECT);
    start_jaeger()?;

    // Initialize the global tracer provider ONCE, pointing at the shared Jaeger's
    // OTLP/HTTP traces endpoint. hydrofoil's init reads this env var and builds
    // the full path verbatim.
    // SAFETY: set before any spans are exported; single-threaded init point.
    unsafe {
        std::env::set_var(
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            format!("http://localhost:{}/v1/traces", jaeger_otlp_port()),
        );
    }
    let otel_guard = desktop_host::telemetry::init_tracing_subscriber();
    *guard = Some(otel_guard);
    eprintln!("[telemetry] shared collector up; engine spans export to Jaeger");
    Ok(())
}

/// Bring up the Jaeger all-in-one collector via its app-scoped compose project.
fn start_jaeger() -> Result<(), String> {
    let fragment = fragments_dir().join("jaeger.yaml");
    let output = std::process::Command::new("docker")
        .args([
            "compose",
            "-p",
            TELEMETRY_PROJECT,
            "-f",
            &fragment.to_string_lossy(),
            "up",
            "-d",
            "--wait",
        ])
        .env("COMPOSE_PROJECT_NAME", TELEMETRY_PROJECT)
        .output()
        .map_err(|e| format!("starting shared telemetry collector: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "shared telemetry collector failed to start ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// The host OTLP/HTTP port the shared Jaeger publishes (matches jaeger.yaml).
fn jaeger_otlp_port() -> String {
    std::env::var("JAEGER_OTLP_HTTP_PORT").unwrap_or_else(|_| "4318".to_string())
}

/// Absolute path to the desktop fragments directory (sibling of `node/`).
fn fragments_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../environments/services/desktop")
}
