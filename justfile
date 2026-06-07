# list all commands by default
_default:
    just --list

# run hydrofoil server
hydro:
    RUST_LOG="hydrofoil=debug" cargo run --bin hydrofoil

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

build-docker:
    docker build -f crates/hydrofoil/Dockerfile -t hydrofoil:dev .
