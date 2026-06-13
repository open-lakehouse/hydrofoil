# S05 — unitycatalog-rs + open-lakehouse: managed read-path correctness

| | |
|---|---|
| Target repos | `unitycatalog-rs` AND `open-lakehouse` (both sibling checkouts required) |
| Depends on | S04 optional (a `getConfig` probe makes the fallback cleaner; not required) |
| Scope | One PR per repo (unitycatalog-rs first, then open-lakehouse consumes it) |
| Findings | A5 (major, source-verified), A6 (major), A10-partial (minor) |

## Mission

Reads of UC catalog-managed Delta tables build a snapshot by merging ratified catalog
commits (from `/delta/v1` loadTable's commit tail) with the `_delta_log` on storage.
The entry points: `unitycatalog-rs/crates/datafusion/src/catalog/delta.rs`
(`build_delta`), `catalog/kernel.rs` (`build_catalog_managed_snapshot`-style snapshot
construction, `to_log_tail`), and a near-verbatim duplicate of the dispatch logic in
`open-lakehouse/crates/hydrofoil/src/catalog/unity.rs`. This session fixes
latest-version resolution, adds the spec-mandated fallbacks, and collapses the
duplication so protocol fixes land once.

Spec references (download first):

```sh
mkdir -p /tmp/uc-specs
curl -sL https://raw.githubusercontent.com/unitycatalog/unitycatalog/main/api/delta.yaml -o /tmp/uc-specs/delta.yaml
curl -sL https://raw.githubusercontent.com/unitycatalog/unitycatalog/main/spec/protocols/ManagedTablesSpec.md -o /tmp/uc-specs/ManagedTablesSpec.md
```

## Findings to fix

### A5 [major, source-verified] Wrong latest-version fallback serves stale snapshots

The pattern
`loaded.latest_table_version.unwrap_or(loaded.metadata.last_commit_version.unwrap_or(0))`
appears in three places:

- `unitycatalog-rs/crates/datafusion/src/catalog/delta.rs:106-108`
- `unitycatalog-rs/crates/datafusion/src/managed/append.rs:56-58`
- `open-lakehouse/crates/hydrofoil/src/catalog/unity.rs:127-129`

Spec: delta.yaml ~lines 783–786 — `latest-table-version` includes data-only commits;
`metadata.last-commit-version` (~lines 825–828) tracks only *metadata-changing*
commits. If the server omits `latest-table-version`, this code caps the snapshot at
the last metadata commit (or 0): silently stale reads, and on the append path the next
commit version collides with an already-ratified version (guaranteed 409 loop).
ManagedTablesSpec line 71: the client must not read beyond the max ratified version —
but it must not invent a lower one either.

**Fix (unitycatalog-rs):** export a single helper, e.g.
`resolve_managed_read_state(loaded: &LoadTableResult) -> Result<(commits, latest)>`:

- For `table-type == MANAGED`: a missing `latest-table-version` is a **hard error**
  (or, only when a commit tail is present, fall back to
  max(commit-tail max version, filesystem `_delta_log` max)); never substitute
  `last-commit-version`.
- `latest-table-version == -1` (and any negative) means "Unity Catalog does not
  manage this table" (ManagedTablesSpec line 465): signal not-catalog-managed so the
  caller routes to the plain filesystem snapshot. Today
  `unitycatalog-rs/crates/datafusion/src/catalog/kernel.rs:39-43` turns `-1` into a
  plan error instead.

Use the helper in `catalog/delta.rs` and `managed/append.rs`.

**Fix (open-lakehouse):** make `hydrofoil/src/catalog/unity.rs` consume the exported
helper and delete its duplicated dispatch/fallback. While there, also dedupe
`to_log_tail` (`unitycatalog-rs/crates/datafusion/src/managed/append.rs:108-125`
duplicates `catalog/kernel.rs:71-96` — export one) and `ensure_trailing_slash`
(3 copies: `managed/create.rs:306`, `managed/append.rs:127`, and
`open-lakehouse/crates/lineage-service/src/writer/unity.rs:62`).

### A6 [major] No fallback when `/delta/v1` loadTable is unavailable

- `unitycatalog-rs/crates/datafusion/src/catalog/delta.rs:94-99` and the duplicate at
  `open-lakehouse/crates/hydrofoil/src/catalog/unity.rs:115-120` — `build_delta`
  calls `load_table` for **every** Delta table (managed and external) and turns any
  error into a hard `DataFusionError`.

Spec: delta.yaml's `UnsupportedTableFormatException` description (~line 1498) says
clients should fall back to the existing UC API; the `getConfig` endpoint
(~lines 53–112) exists for endpoint-support discovery. Against a UC deployment
without `/delta/v1` (production Databricks, older OSS), every Delta read fails even
though the legacy `tables` API already supplied the storage location.

**Fix (unitycatalog-rs):** on 404 / 501 / `UnsupportedTableFormatException`, fall
back to the filesystem snapshot path using the legacy table metadata. Optionally gate
with a one-time-per-client `getConfig` probe if S04 has landed
(`DeltaV1Client::get_config`); otherwise fall back reactively per error and cache the
"unsupported" determination on the client.

### A10-partial [minor]

1. **Staged-commit filename tolerance** —
   `unitycatalog-rs/crates/datafusion/src/catalog/kernel.rs:71-96` (and the buoyant
   kernel's `LogPath::staged_commit`) only accept `<20-digit-version>.<uuid>.json`.
   ManagedTablesSpec line 49 defines that form, but the getCommits example
   (~line 486) shows a bare `<uuid>.json` `file_name`. Tolerate both: synthesize the
   version prefix from `commit.version` when absent. Add a test for both forms.
2. **Time travel not plumbed** —
   `unitycatalog-rs/crates/datafusion/src/catalog/kernel.rs:32-66` supports
   `at_version`, but all callers pass `None` (`catalog/delta.rs:109`, hydrofoil
   `unity.rs:132`). Plumb an optional version through the provider construction
   (hydrofoil's `delta_managed_provider_for` seam) into `at_version`, validating
   requested version ≤ resolved latest. Timestamp-based travel (must use ICT per
   ManagedTablesSpec line 152) may be left as a documented TODO.

## Reference-implementation validation (2026-06-13)

Validated against the UC OSS **Java server** (`~/code/unitycatalog`, HEAD `5a3b69dd`)
and the **Delta reference clients** (`~/code/delta`: kernel
`UCCatalogManagedClient`/`SnapshotBuilderImpl`/`SnapshotManager`, Spark
`UCDeltaCatalogClientImpl`, legacy `SnapshotManagement`). Where this section conflicts
with details above, **this section wins**.

**Confirmed:** A5 is well-founded — *no* reference client ever substitutes
`last-commit-version` for version resolution; the kernel hard-rejects a negative/
missing max catalog version (`SnapshotBuilderImpl.java:111-114`). Time-travel
semantics confirmed (version ≤ max ratified, tail must cover the version, timestamp
travel is ICT-based).

**Corrections to the `-1`/absence semantics (A5/A10):** the OSS server **never
returns `-1`**. For MANAGED Delta tables, `latest-table-version` is *always* set
(`0` right after create, with an empty tail — the create-time `0.json` is never
registered as a commit) and the commit tail is always present (possibly empty). For
EXTERNAL tables the field and `commits` are **omitted entirely**. So
`resolve_managed_read_state` should key "not catalog-managed" off **field absence
and/or `table-type != MANAGED`**, treating a literal `-1` as an equivalent legacy
signal (it appears only in the older preview spec). "Missing for MANAGED = hard
error" stands. Include the post-create state (`latest = 0`, empty tail → snapshot is
the filesystem `0.json` only) in the resolution matrix and tests.

**Corrections to the fallback triggers (A6):** `UnsupportedTableFormatException` is a
**400** with that error type in the Delta envelope — not 404/501. In `/delta/v1`, a
**404 means table-not-found**; do *not* blanket-fallback on 404 or you mask genuinely
missing tables. The trigger set is: typed `UnsupportedTableFormatException` (primary,
matches the Spark reference `UCDeltaCatalogClientImpl.scala:82-86`), plus
route-entirely-missing (404 with **no Delta error envelope**) and 501 for older
deployments. The `getConfig` probe has **zero reference callers** and its endpoint
list is aspirational (it advertises the unimplemented metrics route) — keep the probe
optional/deferred; the reactive typed-error path is the proven mechanism. (getConfig,
if used: requires a `catalog` query param, ignores `protocol-versions` input, returns
`protocol-version: "1.0"`.)

**Additional snapshot-assembly rules the reference enforces (add to the helper):**

- **The commit tail arrives newest-first** (descending) from loadTable/getCommits —
  sort ascending by version defensively before assembly.
- **Version-overlap rule:** when both a published `_delta_log/<v>.json` and a ratified
  staged commit exist for the same version, the **staged commit wins**
  (`SnapshotManager.java:653-689`).
- **Tail-completeness invariants:** for latest reads a non-empty tail must end exactly
  at the resolved latest version; an empty tail (fully backfilled) is valid but the
  filesystem log must then reach the latest version — *error out* rather than silently
  serving an older version (`SnapshotBuilderImpl.java:201-224`,
  `SnapshotManager.java:505-508`). Encode these as assertions in
  `resolve_managed_read_state`.
- **Backfill race:** commits published between the filesystem listing and the catalog
  call can create apparent gaps; the legacy reference re-lists to reconcile
  (`SnapshotManagement.scala:229-261`). Handle or explicitly detect-and-retry.
- **Table-identity check:** validate the loadTable response's table uuid against the
  expected `io.unitycatalog.tableId` and fail typed
  (`UCDeltaTokenBasedRestClient.java:338-349`).

**Filename tolerance (A10) nuance:** the server stores and echoes `file_name`
verbatim (validates non-empty only) — so accept *any* filename and key identity/
ordering on `commit.version`, synthesizing the `<%020d>.<uuid>.json` staged path only
when needed. Note in the PR that this is deliberately more tolerant than the kernel
reference, whose regex (`FileNames.java:53`) would reject the spec's bare-uuid
example.

## Constraints

- Protocol logic lives in unitycatalog-rs; hydrofoil only consumes exported helpers —
  no logic copies remain in open-lakehouse when you're done.
- Crates are unpublished: change APIs freely, no compatibility shims. Update the
  open-lakehouse dependency pin (path/rev in its `Cargo.toml`) as needed.
- Stage changes in each repo and propose per-repo commit messages, but do **not** run
  `git commit` — the user signs commits. Attribute AI work as "AI assisted by Isaac"
  if attribution is included.

## Verification

- Unit tests (unitycatalog-rs): latest-version resolution matrix — present /
  absent-with-tail / absent-without-tail (error) / `-1` (filesystem fallback);
  staged-filename both forms; loadTable-unsupported fallback (mocked 404/501 and
  typed error).
- open-lakehouse builds against the updated unitycatalog-rs and
  `cargo test -p hydrofoil` passes; grep confirms the duplicated fallback pattern
  (`last_commit_version.unwrap_or(0)`) is gone from both repos.
- If a live stack is available (open-lakehouse `environments/`, `just` targets): read
  a managed table end-to-end and confirm the snapshot reflects the latest ratified
  data commit.
