# @open-lakehouse/query-wasm

The in-browser query engine (`wasm/query-wasm`: DataFusion + Delta on
`wasm32`, reading cloud storage directly with UC-vended credentials) packaged
as a `QueryRunner` for the `@open-lakehouse/query-runner` seam. The seam, its
`open_lakehouse.query.v1` contract and the host app live in
[mangrove](https://github.com/open-lakehouse/mangrove); this package and the
engine moved here from there.

## How it plugs in

`registerWasmPreview({ baseUrl })` registers a runner that spawns a **Web
Worker per preview run** (the wasm engine's synchronous kernel bursts must not
run on the main thread), with a **Delta-only** capability probe wired into
`QueryService.supports`. Aborting a run terminates its worker — the engine
holds no cross-run state.

Failures carry a machine-readable `code`:

| code          | meaning                                              | typical handling |
| ------------- | ---------------------------------------------------- | ---------------- |
| `UNSUPPORTED` | outside the engine's v1 envelope (deletion vectors, non-classic checkpoints, unbackfilled managed commits, AWS/R2 storage, zstd/brotli parquet) | fall back        |
| `NETWORK`     | direct storage fetch blocked (CORS or connectivity)  | fall back        |
| `FAILED`      | a real error                                         | surface          |

Pair `isFallbackWorthy` with `createFallbackQueryRunner` (from
`@open-lakehouse/query-runner`) to compose the wasm engine with a host's
server-backed runner.

## Build gating

The worker imports the wasm-bindgen artifact under `wasm/query-wasm/pkg/`,
which is **gitignored** and produced by `just build-query-wasm`; the worker
reaches it through the bare `query-wasm-pkg` import (see `src/pkg.d.ts`), which
the host bundler aliases to `wasm/query-wasm/pkg/query_wasm.js`. Hosts that
ship without the engine alias this whole package to
[`src/stub.ts`](src/stub.ts) (a no-op `registerWasmPreview`) instead.

## Status

The package is private and not yet part of the `node/` npm workspace: it
depends on the published `@open-lakehouse/query-runner` and
`@open-lakehouse/delta-log`, and wiring it in (workspace entry, lockfile,
typecheck in CI) comes with publishing it for mangrove's app to consume.
