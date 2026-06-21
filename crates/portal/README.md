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
- **`src/main.rs`** — serves all services on one axum router via
  `Router::into_axum_service()`.

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
PORTAL_PORT=8080 cargo run -p portal
curl localhost:8080/health      # -> OK
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
