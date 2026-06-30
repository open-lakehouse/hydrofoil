#!/usr/bin/env bash
#
# Run the Tauri desktop app (node/desktop) in dev mode.
#
# The app boots into the environment manager (the picker). Creating + starting an
# environment is what brings UC online: the Tauri backend spawns its own `uc`
# sidecar on a dynamic port (per environment, isolated SQLite + keychain-encrypted
# keys; see `spawn_uc_sidecar` in node/desktop/src-tauri/src/lib.rs) and brings up
# the environment's compose stack (Envoy gateway, etc.). So this script does NOT
# start any standalone Unity Catalog server — that was a legacy escape hatch that
# predated the per-environment sidecar.
#
# What it does:
#   1. Runs `tauri dev`, which boots the desktop Vite server (:3003) and opens the
#      native window.
#   2. Tears down `tauri dev` AND its child Vite server when this script exits
#      (Ctrl-C or otherwise) — `tauri dev` can otherwise orphan the Vite process.
#
# The desktop Vite proxy (node/desktop/vite.config.ts) forwards /api, /mlflow,
# /marimo, and /jaeger to GATEWAY_URL (default http://localhost:9080, the env's
# Envoy gateway) — those work once an environment is running. Override GATEWAY_URL
# to point the proxy elsewhere if needed.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

log() { printf '\033[1;34m[dev-desktop]\033[0m %s\n' "$*"; }

cleanup() {
  local code=$?
  if [[ -n "${TAURI_PID:-}" ]]; then
    log "Stopping Tauri dev (and its Vite server)"
    kill "$TAURI_PID" 2>/dev/null || true
    # tauri dev spawns the Vite server (:3003) as a child; reap whatever still
    # holds the port.
    lsof -tiTCP:3003 -sTCP:LISTEN 2>/dev/null | xargs -r kill 2>/dev/null || true
  fi
  exit "$code"
}
trap cleanup INT TERM EXIT

log "Starting Tauri desktop app (Vite :3003)"
(
  cd "$REPO_ROOT/node/desktop"
  exec npm run tauri:dev
) &
TAURI_PID=$!

wait "$TAURI_PID"
