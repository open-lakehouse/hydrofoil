# 0010 — Unity Catalog catalog-managed Delta table writes

> Status: **Accepted** (2026-06). Implemented in the sibling `unitycatalog-rs` repo
> (`crates/datafusion/src/managed/`), proven end-to-end against the live Java UC OSS server.
> Extends [`0009-lineage-service-unity-catalog-write-path.md`](0009-lineage-service-unity-catalog-write-path.md),
> which deferred the catalog-managed (commit-coordinated) case.

## Context

ADR-0009 designed a Unity Catalog write path for the lineage-service events table but
**deferred catalog-managed tables** — where the catalog, not the filesystem `_delta_log/`, is
the source of truth for commits — to "a future phase needing a UC-aware `LogStore`." We have
now built that write path, and the design differs from what 0009 anticipated.

A catalog-managed Delta table's write lifecycle (UC `/delta/v1` API, served by the **Java**
Unity Catalog OSS server):

- **Create**: `createStagingTable` (UC allocates a table id + managed `storage_location`) →
  client writes `_delta_log/0.json` with the required features/properties → `createTable`
  finalizes the table at version 0.
- **Commit** (per write): write the data file + a *staged* commit
  `_delta_log/_staged_commits/<v>.<uuid>.json` → `updateTable` action `add-commit` (the catalog
  ratifies iff `v == last + 1`, `409` on conflict). Publishing the staged commit to
  `_delta_log/<v>.json` is an optional maintenance step — `loadTable` returns the
  ratified-but-unpublished tail so readers see it regardless.
- **Read**: `loadTable` returns the ratified commit tail + `latest_table_version`; the snapshot
  is assembled from the published log + that tail, capped at the catalog version.

A spike first proved this flow by hand-rolling the Delta log JSON. Investigating the
`buoyant_kernel` fork then revealed it already provides the entire framework, which reshaped
the design.

## Decision

- **Build on the kernel's commit framework, not a delta-rs `LogStore`.** `buoyant_kernel`
  exports a `Committer` trait (`commit` / `is_catalog_committer` / `publish`), `CommitMetadata`
  (staged/published path generation, in-commit timestamps), `PublishMetadata`, and
  `kernel::create_table` (builds Protocol+Metadata, auto-enables `inCommitTimestamp` for
  `catalogManaged`). This is the upstream-blessed seam; a custom `LogStore` (0009's assumption)
  would re-implement what the kernel already does.
- **Port the kernel fork's `UCCommitter` into `unitycatalog-rs`, bound to our `DeltaV1Client`.**
  The fork's experimental `delta-kernel-unity-catalog`/`unity-catalog-delta-rest-client` crates
  contain a working committer + REST client, but are WIP and introduce a *second* UC client. We
  instead port the committer logic (~300 lines, tested) into
  `unitycatalog-rs/crates/datafusion/src/managed/committer.rs`, implementing the kernel
  `Committer` trait directly over our existing `DeltaV1Client` (`updateTable add-commit`). This
  keeps unitycatalog-rs the single UC client. The committer bridges the sync `Committer::commit`
  to our async client via `block_in_place` + `Handle::block_on`, and maps HTTP 409 → kernel
  `CommitResponse::Conflict`.
- **Connector helpers**: `create_managed_table` runs staging → `kernel::create_table` (committed
  by the committer, writes `0.json`) → derive UC-registration properties from the v0 snapshot →
  `createTable`. `append_to_managed_table` loads the catalog-managed kernel snapshot →
  `snapshot.transaction(committer)` → `DefaultEngine::write_parquet` → `add_files` → `commit`.
- **The required table contract** (verified against the running Java server): features
  `catalogManaged` + `vacuumProtocolCheck` + `v2Checkpoint`; `inCommitTimestamp` (writer,
  auto-enabled). `createTable` requests must carry `data-source-format: "DELTA"`. Properties are
  driven from the staging response + the committed snapshot, not hardcoded.
- **Read path is unchanged** — the existing `build_catalog_managed_snapshot` already assembles
  the snapshot from `loadTable`'s tail.

### One required delta-rs change (to be upstreamed)

delta-rs rejected the managed table's `v2Checkpoint` feature when opening it for read/write
(`UnsupportedTableFeatures([V2Checkpoint])`). We allow-listed `TableFeature::V2Checkpoint` in
`crates/core/src/kernel/transaction/protocol.rs`, alongside the pre-existing
`catalogManaged`/`vacuumProtocolCheck` allow-list. Committed to the `roeap/delta-rs` fork
(`dais-demo @ a52e341e`); unitycatalog-rs pins that rev. **Upstream candidate** — open a PR to
delta-rs to add `V2Checkpoint` to the supported-feature set so the pin can move to a released rev.

## Consequences

- The lineage-service (and any consumer) can create + commit a UC catalog-managed Delta table
  through unitycatalog-rs with no hand-rolled log JSON. Proven end-to-end against the live Java
  UC OSS + real S3: create → append → read back the rows.
- **Single UC client** preserved; the connector glue is dependency-light (`buoyant_kernel` +
  `DeltaV1Client`) so it can later move toward `delta-rs/crates/catalog-unity` or adopt the
  buoyant UC crates once they stabilize.
- **Catalog-managed snapshots always require `max_catalog_version`** on the kernel snapshot
  builder — even with an empty unbackfilled tail (e.g. a freshly created table whose only commit
  is the published `0.json`).
- **Client-side publish is deferred.** We ratify but do not copy staged commits to
  `_delta_log/<v>.json`; reads work via the catalog tail. Publishing (and the
  `set-latest-backfilled-version` notification) is a maintenance optimization for a later phase —
  needed before the unbackfilled-commit count grows large.
- **delta-rs is on a fork rev** (`roeap/delta-rs a52e341e`) until the `V2Checkpoint` allow-list
  lands upstream.

## Upstream / follow-up

1. PR the `V2Checkpoint` allow-list to delta-rs; then bump the pin off the fork rev.
2. Wire the lineage-service events table onto `create_managed_table` (the 0009 consumer).
3. Implement client-side publish + backfill notification.
4. Conflict-retry loop on `409` (the spike/round-trip exercised the happy path only).
