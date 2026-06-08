# list all commands by default
_default:
    just --list

# run the hydrofoil server on the host against the local config (which points at
# the compose stack's host-published ports). Override the config path with
# `HYDROFOIL_CONFIG=… just hydro`, or individual fields with `HYDROFOIL__*` env
# vars. Telemetry endpoints stay env-driven (see below); secrets go via UC_TOKEN
# / OPENLINEAGE_API_KEY.
hydro:
    RUST_LOG="hydrofoil=debug" \
    OTEL_EXPORTER_OTLP_TRACES_ENDPOINT="${OTEL_EXPORTER_OTLP_TRACES_ENDPOINT:-http://localhost:10120/v1/traces}" \
    MLFLOW_EXPERIMENT_ID="${MLFLOW_EXPERIMENT_ID:-0}" \
    cargo run --bin hydrofoil -- "${HYDROFOIL_CONFIG:-environments/config/local/hydrofoil.toml}"

# run marimo notebook server
scratch:
    uvx --directory notebooks/ marimo edit --sandbox client.py

env-up env_name="live" *args:
    docker compose -f environments/{{ env_name }}.compose.yaml up -d {{ args }}

env-down env_name="live" *args:
    docker compose -f environments/{{ env_name }}.compose.yaml down {{ args }}

build_policy:
    cd crates/policy && buf generate

push_policy:
    oras push localhost:10100/hydrofoil/plan-policy:latest \
      config/policies/lakehouse.cedar:application/vnd.cedar.policyset.v1 \
      config/policies/lakehouse.cedarschema:application/vnd.cedar.schema.v1 \
      config/policies/lakhouse.entities.json:application/vnd.cedar.entities.v1

# push the policy_demo.py policy set (gate + row filter + column mask) to the local
# OCI registry. Point hydrofoil at it with HYDROFOIL_POLICY_REF=localhost:10100/hydrofoil/demo-policy:latest
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
