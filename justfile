# list all commands by default
_default:
    just --list

# run hydrofoil server
hydro:
    RUST_LOG="hydrofoil=debug" cargo run --bin hydrofoil

# run hydrofoil wired to the full stack: lineage -> Marquez, traces -> MLflow, UC
# (sources environments/.env so OPENLINEAGE_*/OTEL_*/UC_ENDPOINT reach the server)
hydro-full:
    set -a && . environments/.env && set +a && \
    RUST_LOG="hydrofoil=debug" cargo run --bin hydrofoil

# run marimo notebook server
scratch:
    uvx --directory notebooks/ marimo edit --sandbox client.py

# run the UC CRUD demo notebook (host -> localhost:8081 / :9000)
uc-crud:
    uvx --directory notebooks/ marimo edit --sandbox uc_crud.py

# run the UC MANAGED-table notebook against the live AWS bucket (host -> localhost:8081)
uc-managed:
    uvx --directory notebooks/ marimo edit --sandbox uc_managed.py

# run the DuckDB notebook: read + append into a UC managed Delta table (host -> localhost:8081)
uc-duckdb:
    uvx --directory notebooks/ marimo edit --sandbox uc_duckdb.py

# bring up the UC + Postgres + SeaweedFS stack
env-up:
    docker compose -f environments/compose.yaml --profile svc up -d db seaweedfs seaweedfs-init unity-catalog

# bring up the full stack incl. Envoy + MLflow (traces) + Marquez (lineage)
env-up-full:
    docker compose -f environments/compose.yaml --profile svc up -d \
      db seaweedfs seaweedfs-init unity-catalog mlflow \
      marquez-db marquez-api marquez-web envoy

# tear the stack down
env-down:
    docker compose -f environments/compose.yaml --profile svc down

# bring up the minimal local stack: Postgres + UC with a mounted filesystem store
env-local-up:
    docker compose -f environments/compose.local.yaml up -d

# tear the minimal local stack down
env-local-down:
    docker compose -f environments/compose.local.yaml down

# register the example Delta tables in the minimal local stack (file:// tables)
seed-local:
    uv run notebooks/scripts/seed_uc_local.py

services:
    docker compose -p open-lakehouse up -d

run profile *FLAGS:
    docker compose -p open-lakehouse --profile {{ profile }} up {{ FLAGS }}

build_policy:
    cd crates/policy && buf generate

push_policy:
    oras push localhost:10100/hydrofoil/plan-policy:latest \
      config/policies/lakehouse.cedar:application/vnd.cedar.policyset.v1 \
      config/policies/lakehouse.cedarschema:application/vnd.cedar.schema.v1 \
      config/policies/lakhouse.entities.json:application/vnd.cedar.entities.v1

trust-me:
    ./scripts/generate-notation-certs.sh

build-docker:
    docker build -f crates/hydrofoil/Dockerfile -t hydrofoil:dev .
