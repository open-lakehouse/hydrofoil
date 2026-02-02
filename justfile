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
    oras push localhost:10100/hydrofoil/plan-policy:v1 \
      config/policies/hydrofoil.cedar:application/vnd.cedar.policy.v1

init_cert:
    notation cert generate-test --default "hydrofoil.io"

push_cert:
    curl --data-binary "/Users/robert.pack/Library/Application Support/notation/localkeys/hydrofoil.io.crt" -X \
      POST "http://localhost:10100/v2/_zot/ext/notation?truststoreType=ca"

trust-me:
    ./scripts/generate-notation-certs.sh
