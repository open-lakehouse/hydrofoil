# @open-lakehouse/files-wasm

The in-browser volume-files backend: a `FilesRunner` for the
`@open-lakehouse/volume-files` seam that runs `UcFilesEngine` from the
`wasm/query-wasm` engine in a Web Worker per operation and talks to it over the
`portal.files.v1` ConnectRPC contract (`@open-lakehouse/files-connect`). The
seams, the contract bindings and the host app live in
[mangrove](https://github.com/open-lakehouse/mangrove); this package moved here
from there with the engine.

## Build gating

The worker imports the same wasm-bindgen artifact as `@open-lakehouse/query-wasm`
(`wasm/query-wasm/pkg/`, gitignored, produced by `just build-query-wasm`) through
the bare `query-wasm-pkg` import (see `src/pkg.d.ts`). Hosts that ship without
the engine alias this package to [`src/stub.ts`](src/stub.ts) instead.

## Status

The package is private and not yet part of the `node/` npm workspace. It depends
on the published `@open-lakehouse/volume-files` and on
`@open-lakehouse/files-connect`, which is still private in mangrove: publishing
that is a prerequisite for wiring this package in (workspace entry, lockfile,
typecheck in CI).
