# Service review, June 2026 — findings index and remediation sessions

A three-track audit of the open-lakehouse stack and its sibling repositories, run
against the new Unity Catalog OSS Delta APIs and protocol:

- **Spec A** — [`api/delta.yaml`](https://github.com/unitycatalog/unitycatalog/blob/main/api/delta.yaml)
  (`/delta/v1`, kebab-case): the API our `DeltaV1Client` targets.
- **Spec B** — [`spec/protocols/ManagedTablesSpec.md`](https://github.com/unitycatalog/unitycatalog/blob/main/spec/protocols/ManagedTablesSpec.md):
  the managed-tables usage protocol (create / commit / publish / backfill / read flows).

Repos under review (local sibling checkouts; fixes land at the right layer per repo):

| Repo | Path | Role |
| ---- | ---- | ---- |
| open-lakehouse | `~/code/open-lakehouse` | hydrofoil Flight SQL server, datafusion-cedar policy engine, open-lineage producer, lineage-service |
| unitycatalog-rs | `~/code/unitycatalog-rs` | UC Rust client, `/delta/v1` client + models, DataFusion↔UC integration, managed-table committer |
| delta-rs (fork) | `~/code/delta-rs` | DeltaScanNext provider, DeltaDataSink, protocol checker allow-list |

The kernel is the buoyant delta-kernel fork pinned in `open-lakehouse/Cargo.toml`
(`Committer` / `create_table` / `UCCommitter` abstractions for catalog-managed commits).

## How to use this directory

Each file in [`tasks/`](tasks/) is a **self-contained work packet for one session /
one PR**. The file *is* the prompt: paste its full contents into a fresh session (cloud
or local) that has the listed repo checkouts available. No access to the originating
review conversation is needed. The four highest-severity findings (B1, A1, C1, A5) were
spot-checked directly in source during the review and are marked *source-verified* in
their task files.

## Findings summary

Severity: **crit** = exploitable correctness/security defect; **maj** = protocol
violation or real data-loss/correctness bug; **min** = quality/robustness.

### A. UC Delta API / ManagedTablesSpec compliance

The wire models in `unitycatalog-rs/crates/common/src/models/delta/v1.rs` are a
faithful, well-tested mirror of delta.yaml (kebab-case keys, tagged unions, enum
casing, optionality — verified against the spec examples). The problems are protocol
*behavior*, not serialization.

| # | Sev | Finding | Session |
|---|-----|---------|---------|
| A1 | crit | delta-rs commit path can write `_delta_log/<v>.json` directly on catalog-managed tables (unratified commits; fork/corruption risk) | [S02](tasks/S02-delta-rs-catalog-managed-write-gate.md) |
| A2 | maj | Ratified commits never published/backfilled; `set-latest-backfilled-version` never sent | [S03](tasks/S03-uc-rs-commit-lifecycle.md) |
| A3 | maj | No commit retry loop; 409/429/CommitStateUnknown all permanent hard errors | [S03](tasks/S03-uc-rs-commit-lifecycle.md) |
| A4 | maj | Delta error envelope never parsed; typed error predicates always false | [S04](tasks/S04-uc-rs-client-surface.md) |
| A5 | maj | Wrong latest-version fallback (`last-commit-version` substituted) → stale reads; duplicated across repos | [S05](tasks/S05-uc-rs-managed-read-path.md) |
| A6 | maj | `build_delta` hard-fails when `/delta/v1` loadTable unavailable — breaks all reads on non-supporting servers | [S05](tasks/S05-uc-rs-managed-read-path.md) |
| A7 | maj | Staging contract (`required-protocol`/`required-properties`) ignored at create; disk↔catalog metadata divergence | [S06](tasks/S06-uc-rs-create-staging-contract.md) |
| A8 | maj | Staging store S3-only, static creds, expiry ignored, first-prefix selection | [S06](tasks/S06-uc-rs-create-staging-contract.md) |
| A9 | maj | 8 of 12 delta.yaml operations missing from `DeltaV1Client`; metrics never reported | [S04](tasks/S04-uc-rs-client-surface.md) / [S03](tasks/S03-uc-rs-commit-lifecycle.md) |
| A10 | min | `-1` latest-version semantics, staged-filename tolerance, time travel, URL encoding, uuid panic, empty token | [S04](tasks/S04-uc-rs-client-surface.md) / [S05](tasks/S05-uc-rs-managed-read-path.md) |
| A11 | min | `data_source_format` sent in `/delta/v1` createTable although not in the delta.yaml schema (legacy route-level tables-API field; pinned-server workaround); ADR 0010 wording misattributes it to the spec | [S06](tasks/S06-uc-rs-create-staging-contract.md) |

### B. Hydrofoil + Cedar policy integration

Invariants verified to **hold**: per-session `RuntimeEnv` credential isolation
(ADR 0004), UC DDL Cedar-gated through the extension-node chokepoint, deprecated
LogStore methods return not-implemented, and datafusion-cedar is consistently
fail-closed (provider errors, partial-eval failures, untranslatable residuals all
deny).

| # | Sev | Finding | Session |
|---|-----|---------|---------|
| B1 | crit | Subquery scans bypass both Cedar authorization and row/column governance (`visit` vs `visit_with_subqueries`) | [S01](tasks/S01-cedar-gate-coverage.md) |
| B2 | crit | Identity headers trusted blindly; no authn interceptor — identity spoofing | [S09](tasks/S09-hydrofoil-authn-interceptor.md) |
| B3 | maj | Reader/writer ACL uids lower-cased — case-sensitive Cedar matching breaks | [S08](tasks/S08-hydrofoil-identity-facts.md) |
| B4 | maj | Principal-enrichment cache has no TTL — fail-stale over-authorization | [S08](tasks/S08-hydrofoil-identity-facts.md) |
| B5 | maj | `GetFlightInfo` discloses result schema before the Cedar gate | [S07](tasks/S07-hydrofoil-server-hardening.md) |
| B6 | maj | Unmodeled plan nodes default-allow; `allow_statements` left true | [S01](tasks/S01-cedar-gate-coverage.md) |
| B7 | maj | `.expect("encoding failed")` panics on the Flight request path | [S07](tasks/S07-hydrofoil-server-hardening.md) |
| B8 | maj | `do_put_*_update` silently no-op (`Ok(-1)`) — lost writes | [S07](tasks/S07-hydrofoil-server-hardening.md) |
| B9 | maj | All UC vending uses the server token; Cedar is the sole access control (architectural) | [ADR 0011](../../adr/0011-uc-credential-vending-server-token.md), implement after [S09](tasks/S09-hydrofoil-authn-interceptor.md) |
| B10 | min | Ingest pseudo-table authz noise, `\|\|` residual fold gap, ungated metadata RPCs, `status!` internals leak, CPU-runtime IO question, env-var config bridge, no server.rs tests | [S01](tasks/S01-cedar-gate-coverage.md) / [S07](tasks/S07-hydrofoil-server-hardening.md) / [S11](tasks/S11-hydrofoil-lineage-integration.md) |

### C. Lineage integration

Producer envelope shape, facet `_producer`/`_schemaURL`, UUIDv7 run ids, and the
lifecycle test suite are solid.

| # | Sev | Finding | Session |
|---|-----|---------|---------|
| C1 | crit | `columnLineage` facet attached to inputs, never outputs — consumers see no column lineage | [S10](tasks/S10-open-lineage-producer.md) |
| C2 | crit | Column-lineage extraction name-based and scope-blind — unsound for joins/aliases/CTEs | [S10](tasks/S10-open-lineage-producer.md) |
| C3 | crit | Marquez read API `parse_node_id` breaks on URI namespaces (`s3://bucket`) — the namespaces the producer emits | [S13](tasks/S13-lineage-service-read-api.md) |
| C4 | maj | COMPLETE/FAIL events carry plan-time `eventTime` — every run appears 0 ms | [S10](tasks/S10-open-lineage-producer.md) |
| C5 | maj | Terminal-event loss/duplication: execute()-error path, partition-counter desync, prepared-statement runId reuse | [S10](tasks/S10-open-lineage-producer.md) / [S11](tasks/S11-hydrofoil-lineage-integration.md) |
| C6 | maj | Ingest acks 202 then can drop whole flushes (no retry; nullability poison-pill) | [S12](tasks/S12-lineage-service-ingest-durability.md) |
| C7 | maj | Read reconstruction: edge erasure, fabricated run states, constant job name → UI shows one node | [S13](tasks/S13-lineage-service-read-api.md) / [S11](tasks/S11-hydrofoil-lineage-integration.md) |
| C8 | maj | Four endpoints marquez-web calls are missing (404s) | [S13](tasks/S13-lineage-service-read-api.md) |
| C9 | min | Projected-scan schema facet, input dedup, `nominalTime` rename, shutdown drain, search totalCount, 404 on unknown seed, internal-query noise | [S10](tasks/S10-open-lineage-producer.md) / [S13](tasks/S13-lineage-service-read-api.md) |

## Sessions and execution order

| ID | Title | Repo(s) | Depends on | Status |
|----|-------|---------|-----------|--------|
| [S01](tasks/S01-cedar-gate-coverage.md) | Cedar gate coverage (subqueries, unmodeled nodes, statements) | open-lakehouse | — | open |
| [S02](tasks/S02-delta-rs-catalog-managed-write-gate.md) | Reject path-based commits on catalog-managed tables | delta-rs | — | open |
| [S03](tasks/S03-uc-rs-commit-lifecycle.md) | Commit lifecycle: publish, backfill, retry, metrics | unitycatalog-rs | S04 helps, not required | open |
| [S04](tasks/S04-uc-rs-client-surface.md) | Client surface: typed Delta errors + missing endpoints | unitycatalog-rs (+ open-lakehouse follow-through) | — | open |
| [S05](tasks/S05-uc-rs-managed-read-path.md) | Managed read path: latest-version resolution + fallbacks | unitycatalog-rs + open-lakehouse | S04 optional | open |
| [S06](tasks/S06-uc-rs-create-staging-contract.md) | Create/staging contract + staging credentials + A11 | unitycatalog-rs (+ ADR 0010 fix) | S04 (staging creds endpoint) | open |
| [S07](tasks/S07-hydrofoil-server-hardening.md) | Flight server hardening + first server.rs tests | open-lakehouse | S01 | open |
| [S08](tasks/S08-hydrofoil-identity-facts.md) | Identity & facts: ACL case-folding, enrichment TTL | open-lakehouse | — | open |
| [S09](tasks/S09-hydrofoil-authn-interceptor.md) | Authn interceptor (design + implement) | open-lakehouse | S08 | open |
| [S10](tasks/S10-open-lineage-producer.md) | OpenLineage producer correctness | open-lakehouse | — | ✅ done (`01f4c09`) |
| [S11](tasks/S11-hydrofoil-lineage-integration.md) | Hydrofoil lineage integration | open-lakehouse | S10 (drain API) | ✅ done (`4e759dd`) |
| [S12](tasks/S12-lineage-service-ingest-durability.md) | Ingest durability | open-lakehouse | — | open |
| [S13](tasks/S13-lineage-service-read-api.md) | Marquez read API correctness | open-lakehouse | — | ✅ done (`75e9d2c`) |

**Status (2026-06-12).** The lineage track (S10, S11, S13) is complete — all C-series
findings are resolved except the ingest-durability items in S12. Findings C1–C5,
C7–C8 and the C9 minors are fixed; column-level lineage is disabled pending a sound
scope-aware extraction (design sketch recorded in `docs/open-lineage-design.md`).

**Reference validation (2026-06-13).** S03, S05, and S06 were validated against the
reference implementations: the UC OSS Java server (`~/code/unitycatalog`, HEAD
`5a3b69dd`) and the Delta kernel/Spark catalog-managed clients (`~/code/delta`). The
plans are directionally confirmed; corrections and additional obligations are folded
into each task file as a superseding "Reference-implementation validation" section.
Highlights: error dispatch must be by Delta error *type* (two distinct 409s, version
gaps are 400, `CommitStateUnknownException` is never emitted — verify on transport
ambiguity by content comparison); the server never returns `latest-table-version: -1`
(absence/table-type is the routing signal); the staging contract is server-enforced
(ignoring it fails createTable with 400); and `data_source_format` must be
version-gated — current servers reject unknown fields rather than ignoring them.

**Recommended order for the remainder.** First the criticals: S01 (subquery authz
bypass) and S02 (managed-table write gate) — different repos, parallelizable. Then
S04 (it unblocks typed errors for S03 and the staging-credentials endpoint for S06),
then S03/S05/S06 in parallel. S07 after S01 (shared `server.rs` guards), S08 → S09,
S12 anytime.

**Conventions for every session** (also restated in each task file): fix at the right
layer — prefer unitycatalog-rs / delta-rs changes over hydrofoil workarounds; the
crates are unpublished, so change APIs freely and do not add compatibility shims; stage
changes and propose a commit message but do not run `git commit` (the user signs
commits); attribute AI work as "AI assisted by Isaac".
