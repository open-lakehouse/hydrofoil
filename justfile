# list all commands by default
_default:
    just --list

# node/ workspace recipes (UI, uc-client, desktop). Run e.g. `just node ui-dev`
# or `just node desktop-dev`; list them with `just node --list`.
mod node

# run the hydrofoil server on the host against the local config (which points at
# the compose stack's host-published ports). Override the config path with
# `HYDROFOIL_CONFIG=… just hydro`, or individual fields with `HYDROFOIL__*` env
# vars. Telemetry endpoints stay env-driven (see below); secrets go via UC_TOKEN
# / OPENLINEAGE_API_KEY.
hydro:
    RUST_LOG="hydrofoil=debug,deltalake=debug,deltalake_core=debug,buoyant_kernel=debug" \
    OTEL_EXPORTER_OTLP_TRACES_ENDPOINT="${OTEL_EXPORTER_OTLP_TRACES_ENDPOINT:-https://mlflow.openlakehousedemos.dev/v1/traces}" \
    MLFLOW_EXPERIMENT_ID="${MLFLOW_EXPERIMENT_ID:-2}" \
    HYDROFOIL__LINEAGE__URL="${HYDROFOIL__LINEAGE__URL:-https://lineage.openlakehousedemos.dev}" \
    cargo run --bin hydrofoil -- "${HYDROFOIL_CONFIG:-environments/config/live/hydrofoil.toml}"

# run the lineage-service on the host against the DEPLOYED UC (the
# unitycatalog-quickstart ECS stack). Requires UNITY_CATALOG_URL +
# UNITY_CATALOG_TOKEN (and AWS_REGION) in the env — see
# environments/config/deployed/README.md. Override the config path with
# `LINEAGE_CONFIG=…`, or individual fields with `LINEAGE__*` env vars.
lineage-deployed:
    @: "${UNITY_CATALOG_URL:?not set — see environments/config/deployed/README.md}" \
       "${UNITY_CATALOG_TOKEN:?not set — see environments/config/deployed/README.md}"
    RUST_LOG="${RUST_LOG:-lineage_service=debug}" \
    cargo run -p lineage-service -- "${LINEAGE_CONFIG:-environments/config/deployed/lineage-service.toml}"

# open the marimo notebook editor on the demo notebooks
scratch:
    uvx --directory notebooks/ marimo edit --sandbox stage1_marketplace.py

# run the node/ UI (Vite, :3002) against a locally-built Unity Catalog server
# from the sibling unitycatalog-rs checkout. Builds + starts the `uc` server
# (in-memory, REST on :8080), waits for it, then starts the UI with its `/api`
# proxy pointed straight at UC (no Envoy gateway). Reuses a UC server already on
# the port instead of rebinding. Override the checkout/ports with UC_REPO /
# UC_PORT / UI_PORT. Both servers are torn down on exit.
dev-ui:
    ./scripts/dev-ui.sh

# run the Tauri desktop app against a locally-built Unity Catalog server — the
# same backend as `dev-ui`, wrapped in the native window. Starts the `uc` server
# (in-memory, REST on :8080), waits for it, then runs `tauri dev` (desktop Vite on
# :3003) with its `/api` proxy pointed straight at UC (no Envoy gateway). Reuses a
# UC server already on the port. Override with UC_REPO / UC_PORT. UC + Tauri are
# torn down on exit.
dev-desktop:
    ./scripts/dev-desktop.sh

# mint per-user UC tokens for the demo notebooks and write notebooks/.env.
# For each email in UC_DEMO_USERS (default alice@example.com,bob@example.com),
# calls the sibling unitycatalog-quickstart minter and writes UC_TOKEN_<USER>=…
# (the env-var name _demo_auth.py derives from the email's local-part). Requires
# the quickstart's create-user-jwt.sh and a reachable UC server with those users
# (its UC_USERS bootstrap). Override the repo path with UC_QUICKSTART; the minter
# reads UC_SERVER/UC_ADMIN_TOKEN from that repo's .env (source it first, e.g.
# `set -a; source ~/code/unitycatalog-quickstart/.env; set +a`).
mint-demo-tokens:
    #!/usr/bin/env bash
    set -euo pipefail
    quickstart="${UC_QUICKSTART:-$HOME/code/unitycatalog-quickstart}"
    minter="$quickstart/scripts/create-user-jwt.sh"
    [[ -x "$minter" ]] || { echo "minter not found: $minter (set UC_QUICKSTART)" >&2; exit 1; }
    IFS=',' read -ra users <<< "${UC_DEMO_USERS:-alice@example.com,bob@example.com}"
    out="notebooks/.env"
    echo "# Minted by 'just mint-demo-tokens' — do not commit (gitignored)." > "$out"
    echo "UC_DEMO_USERS=${UC_DEMO_USERS:-alice@example.com,bob@example.com}" >> "$out"
    for email in "${users[@]}"; do
      email="$(echo "$email" | xargs)"  # trim
      [[ -n "$email" ]] || continue
      local_part="${email%@*}"
      # Match _demo_auth._env_var_name: non-alphanumerics -> '_', uppercased.
      slug="$(echo "$local_part" | sed 's/[^[:alnum:]]/_/g' | tr 'a-z' 'A-Z')"
      var="UC_TOKEN_${slug}"
      echo "minting token for $email -> $var" >&2
      token="$("$minter" "$email")"
      echo "${var}=${token}" >> "$out"
    done
    echo "wrote $out" >&2

env-up env_name="live" *args:
    docker compose -f environments/{{ env_name }}.compose.yaml up -d {{ args }}

env-down env_name="live" *args:
    docker compose -f environments/{{ env_name }}.compose.yaml down {{ args }}

build_policy:
    cd crates/policy && buf generate

# Regenerate the portal crate's buffa message types + connect-rust service stubs.
# One-time plugin install (local binaries; no remote BSR plugin yet):
#   cargo install --locked connectrpc-codegen     # protoc-gen-connect-rust
#   cargo install --locked protoc-gen-buffa protoc-gen-buffa-packaging
portal-gen:
    cd crates/portal && buf generate
    cargo fmt -p portal

# Regenerate hydrofoil's QueryService buffa message types + connect-rust stubs.
# Proto source lives in proto/hydrofoil-query (root tree); codegen config is the
# crate-local crates/hydrofoil/buf.gen.yaml. Same plugins as `portal-gen`.
hydrofoil-gen:
    cd crates/hydrofoil && buf generate
    cargo fmt -p hydrofoil

push_policy:
    oras push localhost:10100/hydrofoil/plan-policy:latest \
      config/policies/lakehouse.cedar:application/vnd.cedar.policyset.v1 \
      config/policies/lakehouse.cedarschema:application/vnd.cedar.schema.v1 \
      config/policies/lakhouse.entities.json:application/vnd.cedar.entities.v1

# push the demo policy set (gate + row filter + column mask) to the local OCI
# registry (zot on :10100 — see environments/services/zot.yaml). Point hydrofoil
# at it with HYDROFOIL_POLICY_REF=localhost:10100/hydrofoil/demo-policy:latest
push-demo-policy:
    oras push localhost:10100/hydrofoil/demo-policy:latest \
      config/policies/demo.cedar:application/vnd.cedar.policyset.v1 \
      config/policies/lakehouse.cedarschema:application/vnd.cedar.schema.v1 \
      config/policies/demo.entities.json:application/vnd.cedar.entities.v1

trust-me:
    ./scripts/generate-notation-certs.sh

# build a service image from the shared Dockerfile, e.g. `just build-docker hydrofoil`
# or `just build-docker lineage-service`. Set CRATES_PROXY to route crates-io
# through a sparse mirror on networks without direct crates.io access, e.g.
# `CRATES_PROXY=sparse+https://crates-proxy.dev.databricks.com/ just build-docker hydrofoil`.
build-docker bin:
    docker build -f docker/Dockerfile \
      --build-arg BIN={{ bin }} \
      --build-arg CRATES_PROXY="${CRATES_PROXY:-}" \
      -t {{ bin }}:dev .
