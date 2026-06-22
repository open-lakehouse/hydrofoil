# portal

Platform services for the lakehouse, served over **ConnectRPC**:

- **Tags** (`portal.tags.v1`) — governed tag definitions (`TagPoliciesService`) and
  their assignments to entities (`EntityTagAssignmentsService`).
- **Files** (`portal.files.v1`) — file upload/download (streaming) and directory
  operations (`FilesService`).

These APIs were previously, and erroneously, added to the Unity Catalog Rust crate;
they are **not** Unity Catalog APIs. Here they live as standalone Connect APIs.

## Architecture

- **Protos** in `proto/` define the messages and services. Custom options
  (`google.api.http`, `buf.validate`) are intentionally avoided — the ConnectRPC
  toolchain ignores them, routing is by `/<package>.<Service>/<Method>`, and
  request validation is done by hand in the handlers.
- **Code generation** uses [`buffa`](https://github.com/anthropics/buffa) for message
  types (owned struct + zero-copy view) and
  [`connect-rust`](https://github.com/anthropics/connect-rust) for service traits,
  dispatchers, and clients. Generated code is committed under `src/generated/`.
- **`src/store/`** — the `TagStore` / `FileStore` traits and an in-memory
  implementation (`MemoryStore`). State is process-local; swap for a durable backend
  later.
- **`src/service/`** — `AppState` implements the generated service traits, delegating
  to the store.
- **`src/config.rs`** — layered configuration (defaults → file → `PORTAL__*`
  env), matching the other workspace services.
- **`src/main.rs`** — loads the config, builds the stores, and serves all
  services on one axum router via `Router::into_axum_service()`.

## Configuration

Like hydrofoil / lineage-service, portal is configured by layering (lowest
precedence first): struct defaults, an optional config file, then `PORTAL__*`
environment overrides. The config file path is the binary's first positional
argument, or the `PORTAL_CONFIG` env var; with neither, it runs on defaults.

```toml
# portal.toml
port = 8080            # PORTAL__PORT

[files]
backend = "memory"     # "memory" (default) or "unity"; PORTAL__FILES__BACKEND
# For backend = "unity", the UC REST base URL (must end in /api/2.1/unity-catalog/).
# endpoint = "http://unity-catalog:8081/api/2.1/unity-catalog/"
# region   = "us-east-1"
```

The Unity endpoint/token/region are also read from the bare `UNITY_ENDPOINT`,
`UNITY_TOKEN`, and `UNITY_REGION` (or `AWS_REGION`) env vars — the token is a
secret and is never read from the file. Setting `UNITY_ENDPOINT` selects the
`unity` backend even if the file leaves `backend` at its `memory` default.

## Code generation

The three plugins are **local** binaries (a fully-remote BSR pipeline for connect-rust
is not available yet). Install once:

```sh
cargo install --locked connectrpc-codegen                  # protoc-gen-connect-rust
cargo install --locked protoc-gen-buffa protoc-gen-buffa-packaging
```

Then regenerate after editing the protos:

```sh
just portal-gen            # == cd crates/portal && buf generate && cargo fmt -p portal
```

When connect-rust ships a remote BSR plugin, switch the `local:` entries in
`buf.gen.yaml` to `remote: buf.build/anthropics/...`.

## Run it

```sh
cargo run -p portal                       # defaults: :8080, in-memory files
cargo run -p portal -- portal.toml        # from a config file
PORTAL__PORT=9000 cargo run -p portal     # env override
curl localhost:8080/health                # -> OK
```

Connect's unary RPCs are JSON-over-POST, so they are directly curl-able:

```sh
curl -X POST localhost:8080/portal.tags.v1.TagPoliciesService/CreateTagPolicy \
  -H 'content-type: application/json' \
  -d '{"tagPolicy":{"tagKey":"cost_center","values":[{"name":"eng"}]}}'
```

Streaming RPCs (file upload/download) use Connect's framed protocol — drive them with
the generated client (see `tests/connect_e2e.rs`) or `buf curl`.

## Test

```sh
cargo test -p portal           # store unit tests + Connect e2e (unary + streaming)
```
