#!/usr/bin/env bash
#
# Run the node/ UI against a locally-built Unity Catalog server from the sibling
# unitycatalog-rs repo.
#
# What it does:
#   1. Renders a per-run server config (scripts/uc-config.dev.yaml.tmpl) that
#      allow-lists a repo-local, gitignored .uc-data/run.* directory for file://
#      managed storage (kept in-repo so the written data is easy to inspect).
#   2. Builds and starts the `uc` server (in-memory backend, REST API on :8080)
#      from the sibling unitycatalog-rs checkout, using that config.
#   3. Waits for the UC REST API to come up.
#   4. Starts the Vite UI dev server (:3002) with its `/api` proxy pointed
#      straight at the UC server.
#
# Normally the UI's `/api` (and `/mlflow`, `/marimo`) requests go through the
# Envoy gateway from environments/ (default GATEWAY_URL=http://localhost:9080).
# Here there is no Envoy, so we point GATEWAY_URL at the UC server directly. The
# UC server already serves the API at /api/2.1/unity-catalog, which is exactly
# the path the UI's fetch client uses, so the proxy is a straight pass-through.
# (The /mlflow and /marimo embedded-service tabs will 404 — that's expected when
# running UC standalone.)
#
# Both servers are torn down when this script exits (Ctrl-C or otherwise).
#
# Overridable via environment:
#   UC_REPO       sibling unitycatalog-rs checkout (default: ../unitycatalog-rs)
#   UC_PORT       UC server REST port             (default: 8080)
#   UI_PORT       Vite dev server port            (default: 3002)
#   UC_DATA_ROOT  local managed-storage root      (default: repo-local .uc-data/run.*)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UC_REPO="${UC_REPO:-$REPO_ROOT/../unitycatalog-rs}"
UC_PORT="${UC_PORT:-8080}"
UI_PORT="${UI_PORT:-3002}"

UC_BASE="http://localhost:${UC_PORT}"
UC_API="${UC_BASE}/api/2.1/unity-catalog"

log()  { printf '\033[1;34m[dev-ui]\033[0m %s\n' "$*"; }
err()  { printf '\033[1;31m[dev-ui]\033[0m %s\n' "$*" >&2; }

if [[ ! -d "$UC_REPO" ]]; then
  err "Unity Catalog repo not found at: $UC_REPO"
  err "Set UC_REPO to your unitycatalog-rs checkout."
  exit 1
fi
UC_REPO="$(cd "$UC_REPO" && pwd)"

# Per-run local storage root + rendered server config, kept inside a repo-local,
# gitignored `.uc-data/` directory so the data the server writes is easy to find
# and inspect while debugging. The in-memory backend keeps no metadata across
# restarts, but allow-listing this path lets catalogs / schemas / volumes use
# managed storage under file://$UC_DATA_ROOT (it also backs the metastore managed
# root). The path must exist at startup, so create it now (random per run).
UC_DATA_BASE="$REPO_ROOT/.uc-data"
mkdir -p "$UC_DATA_BASE"
UC_DATA_ROOT="${UC_DATA_ROOT:-$(mktemp -d "$UC_DATA_BASE/run.XXXXXX")}"
mkdir -p "$UC_DATA_ROOT"
UC_CONFIG_TEMPLATE="$REPO_ROOT/scripts/uc-config.dev.yaml.tmpl"
# Render the config alongside the data it points at, so both live under the same
# inspectable per-run directory.
UC_CONFIG="$UC_DATA_ROOT/uc-config.yaml"
sed "s|@@UC_DATA_ROOT@@|${UC_DATA_ROOT}|g" "$UC_CONFIG_TEMPLATE" > "$UC_CONFIG"
log "Local storage root : ${UC_DATA_ROOT}"
log "Rendered UC config : ${UC_CONFIG}"

# If something is already serving the UC API on this port, reuse it rather than
# fighting over the bind (e.g. a server left running from a previous session).
UC_PID=""
if curl -fsS -o /dev/null "${UC_API}/catalogs" 2>/dev/null; then
  log "Reusing Unity Catalog server already responding at ${UC_API}"
  log "NOTE: the rendered config only applies to a server started by this script."
  log "      Stop the running server and re-run to pick up ${UC_DATA_ROOT}."
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
  # `cargo run` and `npm`/vite each leave a child process listening on the port,
  # so kill our launcher PIDs *and* whatever still holds the ports we started.
  if [[ -n "${UI_PID:-}" ]]; then
    log "Stopping UI dev server"
    kill "$UI_PID" 2>/dev/null || true
    lsof -tiTCP:"$UI_PORT" -sTCP:LISTEN 2>/dev/null | xargs -r kill 2>/dev/null || true
  fi
  if [[ -n "${UC_PID:-}" ]]; then
    log "Stopping Unity Catalog server"
    kill "$UC_PID" 2>/dev/null || true
    lsof -tiTCP:"$UC_PORT" -sTCP:LISTEN 2>/dev/null | xargs -r kill 2>/dev/null || true
  fi
  # Leave the per-run .uc-data/ directory (config + written data) in place for
  # inspection; it's gitignored and a fresh one is created on the next run.
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
  # Surface an early build/start failure instead of waiting the full timeout.
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

log "Starting UI dev server on http://localhost:${UI_PORT} (API -> ${UC_BASE})"
# Run vite directly (not via `npm run dev`) so $UI_PID is the real vite process
# and the trap can stop it cleanly. The binary is hoisted to the workspace root.
VITE_BIN="$REPO_ROOT/node/node_modules/.bin/vite"
(
  cd "$REPO_ROOT/node/ui"
  # GATEWAY_URL is where vite.config.ts proxies /api (and /mlflow, /marimo).
  # Point it at the UC server so /api/2.1/unity-catalog reaches UC directly.
  exec env GATEWAY_URL="$UC_BASE" "$VITE_BIN" --port "$UI_PORT"
) &
UI_PID=$!

wait "$UI_PID"
