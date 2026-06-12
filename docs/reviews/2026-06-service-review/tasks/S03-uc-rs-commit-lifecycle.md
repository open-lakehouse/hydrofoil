# S03 — unitycatalog-rs: managed-table commit lifecycle (publish, backfill, retry, metrics)

| | |
|---|---|
| Target repo | `unitycatalog-rs` (sibling checkout, e.g. `~/code/unitycatalog-rs`) |
| Depends on | S04 (typed Delta errors) makes the retry logic cleaner but is not required — use HTTP status codes if S04 hasn't landed |
| Scope | One PR |
| Findings | A2 (major), A3 (major), metrics reporting (major), ALTER limitation (documented) |

## Mission

You are working in `unitycatalog-rs`, which implements the Unity Catalog `/delta/v1`
client (`crates/client/src/delta_v1.rs`, models in
`crates/common/src/models/delta/v1.rs`) and the catalog-coordinated managed-table
write path (`crates/datafusion/src/managed/`: `create.rs`, `append.rs`,
`committer.rs` with `UnityCatalogCommitter` built on the buoyant delta-kernel fork's
`Committer` trait). Commits are staged to `_delta_log/_staged_commits/` and proposed
to UC via `updateTable` with an `add-commit` action; UC ratifies them. This session
completes the *post-commit* protocol obligations and makes commits resilient.

Spec references (download first):

```sh
mkdir -p /tmp/uc-specs
curl -sL https://raw.githubusercontent.com/unitycatalog/unitycatalog/main/api/delta.yaml -o /tmp/uc-specs/delta.yaml
curl -sL https://raw.githubusercontent.com/unitycatalog/unitycatalog/main/spec/protocols/ManagedTablesSpec.md -o /tmp/uc-specs/ManagedTablesSpec.md
```

## Findings to fix

### A2 [major] Ratified commits are never published nor backfilled

- `crates/datafusion/src/managed/append.rs:12-15` — explicitly defers publish.
- `crates/datafusion/src/managed/committer.rs:273-286` — `publish()` is implemented
  but has no caller (only the kernel's `Snapshot::publish` would invoke it, and
  nothing in this repo or its consumers calls that).
- `set-latest-backfilled-version` is only sent by the throwaway example
  `crates/datafusion/examples/managed_table_write.rs:290-308`, never by the library.

Spec: ManagedTablesSpec line 75 — after a successful commit the client **must**
publish the ratified commit by copying the committed file into `_delta_log/`, and
after all commits up to a version are published, should notify the server
(`set-latest-backfilled-version`, delta.yaml ~lines 1021–1035). Today every append
leaves its commit unbackfilled forever; the unbackfilled tail grows until the server
returns 429 `ResourceExhaustedException` (delta.yaml ~lines 286–291, 1508) and all
writes to the table hard-fail. Non-catalog-aware readers also never see new versions.

**Fix:** in `append_to_managed_table`, after the transaction commits, call the
post-commit snapshot's `publish(engine, &committer)` and then `update_table` with
`DeltaTableUpdate::SetLatestBackfilledVersion { latest_published_version }`. Both
should be best-effort: a publish/notify failure must not fail the (already ratified)
write — log and continue; the next write or a 429 handler can catch up.

### A3 [major] No retry/conflict/ambiguity handling on commit

- `crates/datafusion/src/managed/committer.rs:221-229` — every non-409 response
  becomes a generic hard error; 409 becomes `ConflictedTransaction`.
- `crates/datafusion/src/managed/append.rs:94-103` — `ConflictedTransaction` is a
  permanent failure; no snapshot rebuild, no retry.

Spec: delta.yaml updateTable 409 — "client should reload the table and retry"
(~lines 278–285); 429 — "client should backfill pending commits before retrying"
(~line 1508); 500 `CommitStateUnknownException` — "client should check the table
state before retrying" (~line 1510). ManagedTablesSpec commit-errors table
(~line 714): rebuild the snapshot and retry the commit with a new version.

**Fix:** wrap the transaction in a bounded retry loop (e.g. configurable, default 3–5
attempts with jittered backoff):

- **409 conflict:** re-run `load_table`, rebuild the snapshot, re-stage the commit at
  the new version, retry.
- **429 resource-exhausted:** run the publish/backfill from A2 for the pending tail,
  then retry.
- **CommitStateUnknown (500):** re-`load_table` and inspect the commit tail for the
  staged file name we proposed — if present, the commit succeeded (return success);
  if absent, retry. Never blind-retry at the same version (duplicate-commit risk) and
  never report failure without checking (falsely failed write).

If S04's typed `DeltaErrorType` parsing has landed, dispatch on it; otherwise
dispatch on HTTP status and leave a `TODO` referencing S04.

### [major] Metrics are never reported after commits

`DeltaReportMetricsRequest` exists (`crates/common/src/models/delta/v1.rs:550-558`)
but there is no client method and no call site. Spec: ManagedTablesSpec line 77 — the
client should send metrics after each successful commit (UC schedules table
maintenance from them); delta.yaml `reportMetrics` (~lines 359–389).

**Fix:** add `DeltaV1Client::report_metrics` (skip if S04 already added it) and call
it best-effort after each successful commit in `append_to_managed_table` — counts are
available from the transaction's add actions.

### [documented limitation] ALTER-style commits are rejected, not propagated

`crates/datafusion/src/managed/committer.rs:134-148`
(`validate_no_alter_table_changes`) hard-fails protocol/metadata/clustering changes.
Spec (ManagedTablesSpec line 73) requires such changes to ride the same commit
request (`set-columns`/`set-properties`/`set-protocol` alongside `add-commit`).
Rejecting is *safer than silently desyncing UC* and stays out of scope here — but
make the limitation explicit: improve the error message to say schema/property
evolution on managed tables is not yet supported, and leave a doc comment pointing at
the spec requirement so the eventual implementation includes the metadata update
actions in the same `updateTable` call.

## Constraints

- Crates are unpublished: change APIs freely, no compatibility shims.
- Best-effort steps (publish, backfill notify, metrics) must never turn a ratified
  commit into a reported failure.
- Stage your changes and propose a commit message, but do **not** run `git commit` —
  the user signs commits. Attribute AI work as "AI assisted by Isaac" if attribution
  is included.

## Verification

- Unit tests with a mocked/wiremocked UC server: (1) 409 → reload/rebuild/retry
  succeeds at the next version; (2) 429 → backfill is attempted, then retry;
  (3) CommitStateUnknown with the staged commit present in the reloaded tail →
  success without re-commit; absent → retry; (4) after a successful append, publish +
  `set-latest-backfilled-version` + `report_metrics` requests are issued; (5) publish
  failure does not fail the append.
- The `#[ignore]` live integration tests in
  `crates/datafusion/tests/managed_table.rs` still compile; run them if a live stack
  is available.
- `cargo test -p datafusion-unitycatalog -p unitycatalog-client` (adjust to actual
  crate names) and `cargo clippy` clean on touched code.
