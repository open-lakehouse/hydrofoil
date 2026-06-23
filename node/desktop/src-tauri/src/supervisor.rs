//! Generic process supervisor for an environment's auxiliary processes.
//!
//! An environment's lifetime owns a set of OS-level processes: the Unity Catalog
//! sidecar (always), and — once service modules land — a Docker Compose project
//! and any uvx sidecars (marimo). This module tracks them all behind one managed
//! slot so `stop_environment` and the app-exit hook can tear the whole set down
//! uniformly, replacing the previous single-purpose `UcSidecar` slot.
//!
//! Teardown is best-effort and never panics: a desktop app can be force-quit, so
//! we prefer to attempt every shutdown and log failures rather than abort.

use std::sync::Mutex;

use tauri_plugin_shell::process::CommandChild;

/// A single process (or process group) bound to the active environment.
pub enum ManagedProcess {
    /// A Tauri-spawned child (UC server, a uvx sidecar). Killed directly.
    Sidecar {
        /// Human-readable label for logging (e.g. `"uc-server"`, `"marimo"`).
        label: String,
        child: CommandChild,
    },
    /// A Docker Compose project, identified by its `-p` project name. Torn down
    /// with `docker compose -p <project> down`. The deterministic project name is
    /// also our crash-recovery handle: a stale project from a prior force-quit is
    /// reconciled by running `down` again on next start.
    // Constructed once service modules are wired into the lifecycle (Phase 4).
    #[allow(dead_code)]
    Compose { project: String },
}

impl ManagedProcess {
    /// Tear this process down. Best-effort — logs and swallows failures so one
    /// stuck process can't block the rest of the teardown.
    fn shut_down(self) {
        match self {
            ManagedProcess::Sidecar { label, child } => {
                if let Err(e) = child.kill() {
                    eprintln!("[supervisor] killing sidecar {label} failed: {e}");
                }
            }
            ManagedProcess::Compose { project } => {
                compose_down(&project);
            }
        }
    }
}

/// Run `docker compose -p <project> down`, removing the project's containers and
/// default network. Best-effort: a missing daemon or already-removed project is
/// logged, not fatal. Used both for teardown and for pre-start orphan reconciliation.
pub fn compose_down(project: &str) {
    match std::process::Command::new("docker")
        .args(["compose", "-p", project, "down", "--remove-orphans"])
        .output()
    {
        Ok(out) if out.status.success() => {}
        Ok(out) => eprintln!(
            "[supervisor] `docker compose -p {project} down` exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => eprintln!("[supervisor] `docker compose -p {project} down` failed: {e}"),
    }
}

/// Managed state: every process owned by the active environment. Drained on stop
/// and on app exit. A fresh environment start pushes new entries after the prior
/// set has been torn down.
#[derive(Default)]
pub struct Supervisor(Mutex<Vec<ManagedProcess>>);

impl Supervisor {
    /// Track a process so it is torn down on the next `shut_down_all`.
    pub fn track(&self, process: ManagedProcess) {
        self.0.lock().unwrap().push(process);
    }

    /// Tear down every tracked process and clear the set. Idempotent — a second
    /// call after the set is drained is a no-op, so the exit hook finds nothing
    /// to do after an explicit `stop_environment`.
    pub fn shut_down_all(&self) {
        let processes: Vec<ManagedProcess> = std::mem::take(&mut *self.0.lock().unwrap());
        for process in processes {
            process.shut_down();
        }
    }
}
