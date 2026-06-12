# S13 — lineage-service: Marquez read-API correctness

| | |
|---|---|
| Target repo | `open-lakehouse` (crates/lineage-service/src/read) |
| Depends on | — (S10/S11 improve the *data* quality; this fixes the read model regardless) |
| Scope | One PR |
| Findings | C3 (critical), C7 (major), C8 (major), C9-partial (minor) |

## Mission

You are working in `open-lakehouse`. `crates/lineage-service/src/read/` serves a
Marquez-compatible REST API under `/api/v1` (routes in `read/http.rs`, model
reconstruction in `read/model.rs` + `read/queries.rs`) by re-folding the stored
OpenLineage event log on each request; the `marquez-web` UI container points at it
(see `environments/`). The reconstruction is lossy in compounding ways — this session
makes the read model truthful.

## Findings to fix

### C3 [critical] `parse_node_id` breaks on URI-style namespaces

- `crates/lineage-service/src/read/model.rs:223-232` — nodeIds are parsed by
  splitting on the first two `:`, so `dataset:s3://bucket:warehouse/t1` parses as
  namespace `s3`, name `//bucket:warehouse/t1`. The producer **deliberately**
  generates `s3://bucket`-style namespaces (`open-lineage` naming follows the
  OpenLineage dataset-naming spec). Consequence: `build_node`
  (`read/queries.rs:434-456`) misses the model lookup and emits synthetic empty
  dataset payloads (epoch timestamps) for every such node, and nodeId-keyed UI
  navigation misroutes. Marquez itself special-cases `scheme://authority` in NodeId
  parsing.

**Fix:** mirror Marquez: when the substring after the first `:` matches a scheme
pattern (`[a-z][a-z0-9+.-]*://`), the namespace extends through the authority; find
the namespace/name boundary after the authority instead of at the next `:`. Tests:
`dataset:s3://bucket:warehouse/t1`, plain `dataset:ns:name`, and a job nodeId.

### C7 [major] Reconstruction erases edges and fabricates run state

1. **Edge erasure** — `read/queries.rs:347-350`: latest-event-wins per job means a
   later event with empty inputs/outputs (FAIL without datasets; the common
   "START carries edges, COMPLETE doesn't" producer pattern) **erases** the job's
   edges. Marquez merges I/O cumulatively per job version. Fix: union edges, or only
   replace when the newer event actually carries dataset refs.
2. **Fabricated run states** — `read/model.rs:87-104`, `read/queries.rs:153-163,
   389-411`: `event_type` is stored but `fold_batch` never reads it; `latestRuns` is
   one synthetic `COMPLETED` run and `/jobs/{job}/runs` echoes it — failed jobs
   render green. Fix: fold per-runId state from the stored `event_type` + `run_id`
   columns; surface the real latest-run state, timestamps, and (once producers emit
   correct terminal eventTimes — S10) duration from START→terminal pairs.

### C8 [major] Endpoints marquez-web calls are missing

- `read/http.rs:27-52` stubs `/stats/*` and `/tags`, but these four are absent and
  404 with red error toasts in the UI:
  - `GET /api/v1/events/lineage` (Events page) — nearly free: a paginated scan of the
    stored `raw_json`.
  - `GET /api/v1/namespaces/{ns}/datasets/{ds}/versions` (dataset detail tab).
  - `GET /api/v1/jobs/runs/{id}/facets` (run detail).
  - `GET /api/v1/column-lineage?nodeId=...` (dataset column view) — fine to return an
    empty result while column lineage is disabled (S10), but it must not 404.

**Fix:** implement `events/lineage` properly; the other three may be honest minimal
implementations over the stored events (versions: fold distinct schema-bearing
events; run facets: from `raw_json`; column-lineage: empty list). Follow the Marquez
REST response shapes the web UI expects (check the marquez-web fetch calls or Marquez
OpenAPI for the field names).

### C9-partial [minor]

1. **Unknown seed nodeId fabricates a node** — `read/queries.rs:244-246` parses the
   seed then discards the result (dead `let _ = (&seed_ns, &seed_name);`);
   `build_node` synthesizes empty payloads (`queries.rs:446, 450`). Return 404 when
   the seed is not in the model, like Marquez.
2. **`search.totalCount` counts the truncated page** — `queries.rs:230-235`
   truncates before counting. Capture `results.len()` before `truncate(limit)`.
3. **Epoch timestamps render as 1970** — `queries.rs:501-505`. Omit unknown
   timestamps or substitute ingestion time.
4. **Per-request full table scan** — `read/mod.rs:11-13`, `queries.rs:57-83`: every
   endpoint re-opens the Delta table and folds *all* events; acknowledged demo
   tradeoff, but unbounded growth degrades silently. Cheap wins now: cache the table
   handle briefly and push projections/filters down; leave indexing as a recorded
   follow-up.
5. **Silent empty UI under UC delta modes** — `read/mod.rs:86-89`: with
   `delta.mode = unity-*` the read path still reads the local `delta.table_path`
   (likely empty) while ingest writes to UC. Log a prominent startup warning when
   mode ≠ local, or wire the read path through the same locator.

## Constraints

- Match the Marquez REST contract the `marquez-web` image expects — when in doubt,
  check the Marquez OpenAPI spec / web client code rather than inventing shapes.
- Crates are unpublished: change APIs freely, no compatibility shims.
- Stage your changes and propose a commit message, but do **not** run `git commit` —
  the user signs commits. Attribute AI work as "AI assisted by Isaac" if attribution
  is included.

## Verification

- `cargo test -p lineage-service` with new tests: URI-namespace nodeId round-trip;
  edge union across START(edges)+COMPLETE(no edges); run-state folding
  (START→COMPLETE, START→FAIL, START-only=RUNNING); the four new endpoints return
  200 with plausible shapes; unknown seed → 404; totalCount.
- Live-stack check if available (`environments/`, `just`): click through
  marquez-web — graph, dataset detail, run detail, events page — no red error
  toasts; a failed query renders as failed.
