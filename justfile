# list all commands by default
_default:
    just --list

# run local tauri app
app:
    npm run tauri dev -w app

# run hydrofoil server
hydro:
    RUST_LOG="hydrofoil=debug" cargo run --bin hydrofoil

# run marimo notebook server
scratch:
    uvx --directory notebooks/ marimo edit --sandbox client.py

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
