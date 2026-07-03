# list all commands by default
_default:
    just --list

# node/ workspace recipes (UI, uc-client, desktop). Run e.g. `just node ui-dev`
# or `just node desktop-dev`; list them with `just node --list`.
mod node

# run the hydrofoil server on the host. Point it at a config with
# `HYDROFOIL_CONFIG=… just hydro`, or override individual fields with
# `HYDROFOIL__*` env vars. Telemetry endpoints stay env-driven (see below);
# secrets go via UC_TOKEN / OPENLINEAGE_API_KEY.
hydro:
    RUST_LOG="hydrofoil=debug,deltalake=debug,deltalake_core=debug,buoyant_kernel=debug" \
    OTEL_EXPORTER_OTLP_TRACES_ENDPOINT="${OTEL_EXPORTER_OTLP_TRACES_ENDPOINT:-https://mlflow.openlakehousedemos.dev/v1/traces}" \
    MLFLOW_EXPERIMENT_ID="${MLFLOW_EXPERIMENT_ID:-2}" \
    HYDROFOIL__LINEAGE__URL="${HYDROFOIL__LINEAGE__URL:-https://lineage.openlakehousedemos.dev}" \
    cargo run --bin hydrofoil -- "${HYDROFOIL_CONFIG:?set HYDROFOIL_CONFIG to a hydrofoil.toml}"

build_policy:
    cd crates/policy && buf generate

# Regenerate the portal crate's buffa message types + connect-rust service stubs.
# One-time plugin install (local binaries; no remote BSR plugin yet):
#   cargo install --locked connectrpc-codegen     # protoc-gen-connect-rust
# cargo install --locked protoc-gen-buffa protoc-gen-buffa-packaging
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

# push the demo policy set (gate + row filter + column mask) to a local OCI
# registry (e.g. zot on :10100). Point hydrofoil at it with
# HYDROFOIL_POLICY_REF=localhost:10100/hydrofoil/demo-policy:latest
push-demo-policy:
    oras push localhost:10100/hydrofoil/demo-policy:latest \
      config/policies/demo.cedar:application/vnd.cedar.policyset.v1 \
      config/policies/lakehouse.cedarschema:application/vnd.cedar.schema.v1 \
      config/policies/demo.entities.json:application/vnd.cedar.entities.v1

trust-me:
    ./scripts/generate-notation-certs.sh

# build a service image from the shared Dockerfile, e.g. `just build-docker hydrofoil`.
# Set CRATES_PROXY to route crates-io through a sparse mirror on networks without
# direct crates.io access, e.g.
# `CRATES_PROXY=sparse+https://crates-proxy.dev.databricks.com/ just build-docker hydrofoil`.
build-docker bin:
    docker build -f docker/Dockerfile \
      --build-arg BIN={{ bin }} \
      --build-arg CRATES_PROXY="${CRATES_PROXY:-}" \
      -t {{ bin }}:dev .
