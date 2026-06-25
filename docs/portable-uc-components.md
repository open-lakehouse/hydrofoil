# Portable Unity Catalog components — technical strategy

> A record of the strategy for carving the Unity Catalog UI out of `hydrofoil`
> into reusable pieces other projects (notably `mangrove`) can pull in, without
> rebuilding a full UI anywhere else. Source material for the prototype that
> inverts the API-client dependency and the cross-repo work it implies.

## Why

`hydrofoil` contains a working Unity Catalog browser — a catalog tree, per-entity
detail panes, create/edit/delete dialogs — built on React, TanStack Query, and a
shadcn/Radix component layer. The data it shows comes from the Rust Unity Catalog
server in the sibling `mangrove` repo (the REST API under
`/api/2.1/unity-catalog`).

There is a reasonable argument that some of this UI should be co-located with the
server that owns the API. But we explicitly **do not** want to grow a second
full-blown UI in `mangrove`. The goal is narrower and more durable: extract a
small set of components — and the client they talk to — that another project can
pull in the same way it pulls in a shadcn component, while the bulk of the
application UI stays in `hydrofoil`.

The question this doc answers: *is the UC UI too deeply integrated to extract, or
can we make it headless enough to travel?* The answer is that the rendering layer
was never the hard part — the coupling lives entirely in how the API client is
constructed. Remove that one coupling and the rest follows.

## Design principles

1. **Headless first.** A reusable component takes its data dependency by
   injection, not by importing a module singleton. The host decides the base
   URL, the transport, and the auth — the component decides nothing about the
   network.
2. **Own the client where the spec lives.** The wire client is generated from
   proto, and `mangrove` owns the proto. The client should be generated and
   shipped from `mangrove`, consumed by everyone else as an artifact.
3. **Distribute visuals by copy, logic by package.** Presentational components
   are meant to be forked and restyled — ship them shadcn-style (copy-in).
   Data-fetching conventions are a contract — ship them as a versioned package.
4. **Auth is a seam, not a default.** The client must support both same-origin
   cookie auth and host-supplied bearer tokens, chosen by the embedder. Neither
   is hard-coded.
5. **Don't rebuild the UI in `mangrove`.** Extraction produces shareable parts,
   not a second application.

## The three layers (and which one is actually coupled)

The UC UI in `node/ui` stratifies cleanly:

- **Presentational** — `components/catalog/detail/*`, `Meta`/`MetaGrid`,
  `TreeRow`, `ListStates`/`DetailStates`, and the shadcn primitives in
  `components/ui/`. These are props → JSX. Portable today, modulo their
  dependence on the consumer's shadcn primitives and Tailwind theme tokens.
- **Data orchestration** — `lib/uc/queries.ts` and `lib/uc/mutations.ts`. The
  genuinely valuable conventions: cursor pagination, list→detail cache seeding,
  predicate-based invalidation. Portable *in principle*, but every hook closes
  over `$api`.
- **Transport / client** — `lib/api.ts` builds `$api` as a **module-level
  singleton** from a hard-coded base path and `lib/client/registry.ts`'s
  `clientFetch`. `registry.ts` is already a deliberate, framework-agnostic
  late-binding seam for the transport.

The only real obstacle is that `$api` is constructed at import time, not
injected. Anything importing `useCatalogs` transitively pulls in `import.meta.env`
(Vite-only), a fixed UC base path, and the generated `@open-lakehouse/uc-client`
types. The components are not "too functional" to extract — they are one
indirection away.

## Critical decision 1 — Replace the OpenAPI client with the trestle WASM client

The sibling `trestle` repo recently grew a WASM-compatible client generator
(`olai-codegen` + `olai-http-wasm`). It turns proto with `google.api.http`
annotations into a `#[wasm_bindgen]` client exposing a plain JS surface:

```ts
import init, { UnityCatalogClient } from "@unitycatalog/wasm-client";
await init();
const client = new UnityCatalogClient(baseUrl);
await client.catalogs().list({ max_results: 100 });
```

It speaks REST/JSON over the same endpoints `lib/api.ts` hits today, so it is a
drop-in for the wire layer. `mangrove` already runs gnostic-openapi codegen from
its UC proto, so generating a WASM client from the same proto slots into existing
tooling.

Adopting it **collapses the package count from three to two**:

- It eliminates the `@open-lakehouse/uc-client` types package and the
  `openapi-fetch` / `openapi-react-query` dependencies. Request logic is written
  once in Rust and tested there, not re-derived per JS consumer.
- The `registry.ts` fetch seam mostly dissolves — base URL and transport config
  move into the client constructor. A thin seam survives, but only for auth (see
  decision 3).

**Ownership:** the WASM client is generated and published from `mangrove`, where
the proto lives. `hydrofoil` and any other consumer depend on the published npm
artifact. This is the concrete answer to "should this live in `mangrove`?" — the
*client* should; the *application UI* should not.

## Critical decision 2 — Invert `$api` from singleton to injected dependency

This is the crux, and it is independent of decision 1 — it is the right move
whether the injected client is the OpenAPI client or the WASM client.

Today:

```ts
// lib/api.ts — constructed at import time
export const $api = createQueryClient(fetchClient);  // hard-coded base URL, Vite env
```

Target:

```tsx
const { client } = useUnityCatalog();   // from <UnityCatalogProvider client={…}>
```

A `UnityCatalogProvider` holds the client instance; `lib/uc/queries.ts` and
`mutations.ts` read it via `useUnityCatalog()` instead of importing `$api`. The
React Query conventions stay byte-for-byte identical — only their source of the
client changes. This removes `import.meta.env` and the fixed base path from the
portable surface entirely.

The transport registry (`lib/client/registry.ts`) already proves the team thinks
in these terms — it injects the *transport* for the ConnectRPC services via
`registerTransport`. This decision lifts the same pattern up to the UC client.

Because the inversion is client-agnostic, it can be **prototyped now against the
current OpenAPI client** — proving the seam with zero behavior change — and the
injected client swapped to the WASM client once decision 1 lands.

## Critical decision 3 — Auth is an injectable hook, supporting both modes

The two repos have incompatible auth assumptions today:

- `trestle`'s WASM client hard-codes `credentials: "include"` — it relies on the
  *browser session* (same-origin cookies). No token injection on the WASM path.
- `mangrove`'s server expects a reverse-proxy model: a pluggable `Authenticator`
  trait, default `AnonymousAuthenticator` (allow-all), designed to read an
  injected identity header. Authz is a `Policy` trait, today `ConstantPolicy`
  (allow-all); no Cedar yet, despite the bridge crates in `hydrofoil`.

Rather than pick one, the WASM client takes an **injectable auth hook**, so the
embedder chooses:

- **Cookie mode** — same-origin or CORS-with-credentials; pass `credentials:
  "include"`, no hook. Works as the WASM client does today.
- **Bearer mode** — the host app holds an OAuth/PAT token and supplies a hook
  returning `{ Authorization: "Bearer …" }`. The token plumbing lives in the
  host, never in the client or the components.

This requires one small, additive change *upstream in `trestle`*: the WASM
transport (`olai-http-wasm`) must expose a per-request header hook, and the
generated constructor must accept an options object carrying it. This is the
`as_header` / `with_auth` feature; it is the first task in the sequence because
the client's shape gates everything downstream.

**Server-side corollary (not a client blocker):** real bearer embedding needs
`mangrove` to grow a non-anonymous `Authenticator` that validates the token, and
eventually a real `Policy`. This is parallel work, not a prerequisite for the
prototype.

## Resulting package topology

Two artifacts, plus the host's own shadcn setup:

| Artifact | Owner | Distribution | Contents |
| --- | --- | --- | --- |
| `@unitycatalog/wasm-client` | `mangrove` | npm package | WASM client generated from UC proto; injectable auth hook |
| UC component layer | `hydrofoil` (or a shared UI pkg) | headless hooks as a package; presentational components as a shadcn registry | `lib/uc/*` conventions injecting the client; `components/catalog/*` copy-in |

Compared with the pre-WASM plan (three packages: generated types + headless hooks
+ component registry), this is strictly simpler: the client lands where its
source of truth is, and the only net-new cross-repo work is one additive feature
in `trestle`.

## What this does *not* simplify

The WASM client makes *data fetching* portable; it does nothing for *rendering*.
The copy-in components still assume the consumer runs shadcn + Tailwind v4 with
the same CSS-variable theme tokens (`text-muted-foreground`, etc.). A non-shadcn
consumer can only use the headless-hooks layer. Likewise, selection and expansion
state (`components/catalog/selection.ts`, `ExpansionContext.tsx`) currently read
TanStack Router and `sessionStorage`; for a clean carve these should become
props/callbacks rather than router reads — another headless-ification, tracked
separately from the client work.

There is also a table-duplication worth noting (future cleanup, not blocking).
The repo has two unrelated tables: `components/data-grid/` — a virtualized,
Arrow-backed, **UC-free** primitive that is the genuine reusable data grid — and
`components/storage/StorageTable.tsx`, a UC-specific admin grid that hand-rolls
its own `<table>` and is bound to UC types/hooks/detail panes. `StorageTable` is
not a low-level component others could depend on UC-free; it is UC all the way
down and lives inside the UC module. The clean future factoring is to make
`StorageTable` a thin UC adapter over a *generic* presentational table (either
generalize `data-grid` beyond Arrow or extract a small `<DataTable>` core), so
the generic table is the shared primitive and UC only supplies column/row
config. That is a behavior-changing refactor, deliberately out of scope for the
no-behavior-change carve-out.

## Sequencing

1. **`trestle`** — add the injectable auth-header hook (`as_header` / `with_auth`)
   to the WASM client. Gates everything.
2. **`mangrove`** — generate and publish `@unitycatalog/wasm-client` from the UC
   proto.
3. **`hydrofoil`** — invert `$api` → `UnityCatalogProvider` / `useUnityCatalog()`.
   Prototype against the current OpenAPI client first (zero behavior change),
   then swap the injected client to the WASM client.
4. **`mangrove`** — real `Authenticator` (bearer validation) and eventual
   `Policy`.
5. Extract the headless hooks package and the shadcn component registry once 3 is
   proven.

The prototype in step 3 is fully contained in this repo and de-risks the entire
plan, so it runs first locally even though it depends on 1–2 to reach production.

---

*This document was AI-assisted by Isaac.*
