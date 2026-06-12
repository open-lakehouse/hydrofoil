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
