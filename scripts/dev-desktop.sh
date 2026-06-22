#!/usr/bin/env bash
#
# Run the Tauri desktop app (node/desktop) against a locally-built Unity Catalog
# server from the sibling unitycatalog-rs repo — the same backend dev-ui.sh uses,
# just wrapped in the native window instead of a browser tab.
#
# What it does:
#   1. Renders a per-run server config (scripts/uc-config.dev.yaml.tmpl) that
#      allow-lists a repo-local, gitignored .uc-data/run.* directory for file://
#      managed storage.
#   2. Builds and starts the `uc` server (in-memory backend, REST API on :8080)
#      from the sibling unitycatalog-rs checkout, using that config.
#   3. Waits for the UC REST API to come up.
#   4. Runs `tauri dev`, which boots the desktop Vite server (:3003) and opens the
#      native window. The desktop Vite proxy forwards /api straight to the UC
#      server (no Envoy gateway), exactly as dev-ui.sh does for the browser UI.
#
# There is no Envoy here, so GATEWAY_URL points at the UC server directly. The UC
# server serves the API at /api/2.1/unity-catalog — the path the UI's fetch client
# uses — so the proxy is a straight pass-through. (The /mlflow and /marimo tabs
# will 404 when running UC standalone; that's expected.)
#
# Both the UC server and the Tauri/Vite processes are torn down when this script
# exits (Ctrl-C or otherwise).
#
# Overridable via environment:
#   UC_REPO       sibling unitycatalog-rs checkout (default: ../unitycatalog-rs)
#   UC_PORT       UC server REST port             (default: 8080)
#   UC_DATA_ROOT  local managed-storage root      (default: repo-local .uc-data/run.*)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UC_REPO="${UC_REPO:-$REPO_ROOT/../unitycatalog-rs}"
UC_PORT="${UC_PORT:-8080}"

UC_BASE="http://localhost:${UC_PORT}"
UC_API="${UC_BASE}/api/2.1/unity-catalog"

log()  { printf '\033[1;34m[dev-desktop]\033[0m %s\n' "$*"; }
err()  { printf '\033[1;31m[dev-desktop]\033[0m %s\n' "$*" >&2; }

if [[ ! -d "$UC_REPO" ]]; then
  err "Unity Catalog repo not found at: $UC_REPO"
  err "Set UC_REPO to your unitycatalog-rs checkout."
  exit 1
fi
UC_REPO="$(cd "$UC_REPO" && pwd)"

# Per-run local storage root + rendered server config, kept inside a repo-local,
# gitignored `.uc-data/` directory (see dev-ui.sh for the rationale).
UC_DATA_BASE="$REPO_ROOT/.uc-data"
mkdir -p "$UC_DATA_BASE"
UC_DATA_ROOT="${UC_DATA_ROOT:-$(mktemp -d "$UC_DATA_BASE/run.XXXXXX")}"
mkdir -p "$UC_DATA_ROOT"
UC_CONFIG_TEMPLATE="$REPO_ROOT/scripts/uc-config.dev.yaml.tmpl"
UC_CONFIG="$UC_DATA_ROOT/uc-config.yaml"
sed "s|@@UC_DATA_ROOT@@|${UC_DATA_ROOT}|g" "$UC_CONFIG_TEMPLATE" > "$UC_CONFIG"
log "Local storage root : ${UC_DATA_ROOT}"
log "Rendered UC config : ${UC_CONFIG}"

# Reuse a UC server already serving the API on this port rather than rebinding.
UC_PID=""
if curl -fsS -o /dev/null "${UC_API}/catalogs" 2>/dev/null; then
  log "Reusing Unity Catalog server already responding at ${UC_API}"
  log "NOTE: the rendered config only applies to a server started by this script."
else
  log "Building + starting Unity Catalog server from ${UC_REPO} (port ${UC_PORT})..."
  log "First build can take several minutes."
  (
    cd "$UC_REPO"
    exec env RUST_LOG="${RUST_LOG:-info}" \
      cargo run --quiet -p unitycatalog-cli -- server --rest --port "$UC_PORT" \
        --config "$UC_CONFIG"
  ) &
  UC_PID=$!
fi

cleanup() {
  local code=$?
  if [[ -n "${TAURI_PID:-}" ]]; then
    log "Stopping Tauri dev (and its Vite server)"
    kill "$TAURI_PID" 2>/dev/null || true
    # tauri dev spawns the Vite server (:3003) as a child; reap whatever still
    # holds the port.
    lsof -tiTCP:3003 -sTCP:LISTEN 2>/dev/null | xargs -r kill 2>/dev/null || true
  fi
  if [[ -n "${UC_PID:-}" ]]; then
    log "Stopping Unity Catalog server"
    kill "$UC_PID" 2>/dev/null || true
    lsof -tiTCP:"$UC_PORT" -sTCP:LISTEN 2>/dev/null | xargs -r kill 2>/dev/null || true
  fi
  exit "$code"
}
trap cleanup INT TERM EXIT

# Wait for the UC REST API to answer (covers the cold-build case above).
log "Waiting for Unity Catalog API at ${UC_API} ..."
for _ in $(seq 1 300); do
  if curl -fsS -o /dev/null "${UC_API}/catalogs" 2>/dev/null; then
    log "Unity Catalog is up."
    break
  fi
  if [[ -n "$UC_PID" ]] && ! kill -0 "$UC_PID" 2>/dev/null; then
    err "Unity Catalog server exited before becoming ready. See output above."
    exit 1
  fi
  sleep 2
done

if ! curl -fsS -o /dev/null "${UC_API}/catalogs" 2>/dev/null; then
  err "Timed out waiting for Unity Catalog API at ${UC_API}"
  exit 1
fi

log "Starting Tauri desktop app (Vite :3003, API -> ${UC_BASE})"
# GATEWAY_URL is where the desktop vite.config.ts proxies the UC REST API (and
# /mlflow, /marimo). Point it at the UC server so /api/2.1/unity-catalog reaches
# UC directly. `tauri dev` runs `beforeDevCommand` (npm run dev) which inherits
# this env, so the proxy is configured without touching any config file.
#
# OPEN_LAKEHOUSE_UC_URL is what the Tauri *Rust* backend (the in-process portal
# Files + hydrofoil QueryService executors) resolves UC against — it does not go
# through Vite. Point it at the same UC server. Tags/Files/Query now run in-process
# in the backend, so they no longer need the QUERY_URL/gateway proxy; only the UC
# REST client still goes over HTTP to ${UC_API}.
(
  cd "$REPO_ROOT/node/desktop"
  exec env GATEWAY_URL="$UC_BASE" \
    OPEN_LAKEHOUSE_UC_URL="${UC_API}/" \
    npm run tauri:dev
) &
TAURI_PID=$!

wait "$TAURI_PID"
