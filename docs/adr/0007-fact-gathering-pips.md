# 0007 — Fact-gathering PIPs: resource/catalog facts, the fact store, and the trait-now/impl-later pattern

> Status: **Accepted** (2026-06). Implemented across a staged series (neutral
> fact types → fact store → resource-entity folding → catalog capture → taint
> recording). Realizes the **PIP catalog** (decision 4) of
> [`platform-policy-architecture.md`](../platform-policy-architecture.md) and the
> locality classification of
> [ADR-0006](0006-policy-fact-locality-and-session-state.md). The principal/identity
> PIP is split into its own record, [ADR-0008](0008-principal-identity-resolution.md).

## Context

[ADR-0006](0006-policy-fact-locality-and-session-state.md) fixed the *policy* for
how facts reach a Cedar decision (locality classification; re-evaluate-fully).
What remained was the *mechanism*: a Cedar decision is a function of a `Request`
plus an `Entities` store, and today the only request-time entity folded in is the
principal — so `resource.*` attributes (owner, readers/writers, classification
tags) resolve only from the static OCI bundle, never from live catalog metadata.
The `fact_gathering_walkthrough` example mocks these facts; nothing real gathers
them.

Three constraints shape the design:

1. **Layering.** `datafusion-cedar` must not depend on `unitycatalog-*` or on the
   `deltalake-datafusion` crate; it is the reusable, catalog-agnostic policy half.
   So catalog facts cannot cross into it as UC types.
2. **Locality (ADR-0006).** Catalog facts are *local-ephemeral* — gathered per
   query, folded into one evaluation, never persisted. Taints are
   *shared-session-scoped* — established at one PEP, read at a later one.
3. **No tags API.** The Unity Catalog `Table` model in our fork
   (`unitycatalog_common::models::tables::v1::Table`) exposes `owner`,
   `properties: HashMap<String,String>`, `comment`, and `columns` (each with a
   `comment`) — but **no first-class tags/classification field and no tag API**.
   Classification must be derived another way.

## Decision

**Neutral fact types in `datafusion-cedar`, catalog translation in the host.**
Introduce `TableFacts` (owner, readers, writers, tags, column_tags) and a
per-query `CatalogFactSink` (keyed by a normalized `TableReference`) in
`datafusion-cedar::facts`. The host (hydrofoil) translates its catalog's `Table`
into `TableFacts` and writes them into the sink during catalog resolution; the
Cedar layer reads the sink and folds a `Table` resource entity carrying those
attributes into the request-time `Entities`. Facts flow host → neutral type →
`datafusion-cedar`; no crate cycle.

**A single `EvalContext` seam.** Per-query, non-plan facts (the `CatalogFactSink`,
the correlation id, and — behind `governance` — the session fact store) travel
into the `Policy` trait methods as one `EvalContext`, so future fact sources grow
in one place rather than in the trait signature.

**Trait-now, simple-impl-v1 for every fact source.** Each PIP is a trait so the
real backend is a drop-in:
- `TagProvider` (classification): v1 `ConventionTagProvider` reads UC
  `Table.properties` (`tags`/`classification`, `tag.<col>`) and column `comment`
  markers (`[tags: …]`); a future HTTP/gRPC classification service is the same
  trait.
- `FactStore` (session taint ledger): v1 `InMemoryFactStore` (`DashMap` keyed by
  correlation id); a future Redis / central-PDP backend is the same trait.

**Facts-by-convention is the v1 source of tags** (forced by the no-tags-API
constraint), behind the `TagProvider` seam so it is replaceable without touching
the policy layer.

## Consequences

- The static OCI bundle stops being the only source of `resource.*` attributes;
  live catalog metadata now informs decisions. (The principal side is closed by
  [ADR-0008](0008-principal-identity-resolution.md).)
- `Policy::is_allowed` / `table_policy` gain an `&EvalContext` parameter. Layer-2
  governance deliberately keeps the resource *unknown* (to preserve residuals),
  so it consumes facts only for taint recording and future tag→column-mask
  expansion, not for entity folding.
- Per-query freshness without per-query allocation: the sink is keyed by
  `TableReference` and overwritten on re-resolution, so a session-owned sink stays
  correct (a table absent from the current plan is simply never read).
- The convention encoding is a v1 affordance, not a contract; the external
  classification service replaces it behind `TagProvider` with no policy-layer
  change.
- `governance` gates the `FactStore` and taint recording; catalog entity folding
  (Layer 1) is unconditional.
