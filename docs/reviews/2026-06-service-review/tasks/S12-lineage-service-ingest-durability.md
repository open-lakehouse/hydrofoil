# S12 — lineage-service: ingest durability

| | |
|---|---|
| Target repo | `open-lakehouse` (crates/lineage-service) |
| Depends on | — |
| Scope | One PR |
| Findings | C6 (major), C9-partial / hygiene (minor) |

## Mission

You are working in `open-lakehouse`. `crates/lineage-service` ingests OpenLineage
events over HTTP (`src/http.rs`, returns 202), converts them
(`src/ingest/converter.rs`), buffers them (`src/writer/buffered.rs`), and flushes to
sinks (Delta/Iceberg/UC — `src/writer/{delta,iceberg,unity*}.rs`, Arrow schema in
`src/writer/schema.rs`). The sinks themselves are careful (not-found/already-exists
handling, per-append credential vending); the buffer layer is where acked events get
lost. This session makes flush loss-resistant and removes the poison-pill.

## Findings to fix

### C6 [major] Ack-then-lose: unconditional buffer clear + nullability poison-pill

1. `crates/lineage-service/src/writer/buffered.rs:144-170` — events are acked (202)
   once on the in-memory channel; `flush()` then **clears the buffer
   unconditionally** even when a sink write fails (delta commit conflict, transient
   S3 error) — the whole flush is dropped with only an error log. No retry, no
   dead-letter. (A hard crash additionally loses up to channel_capacity +
   buffer_size ≈ 1100 acked events; graceful shutdown drains.)
2. `crates/lineage-service/src/writer/schema.rs:16-21` declares `event_time`
   non-nullable, but `append_run/append_job/append_dataset` call
   `etime.append_null()` when the timestamp is unset (`schema.rs:500-504, 553-557,
   603-607`). If any event survives conversion with an unset timestamp,
   `RecordBatch::try_new` fails and **all** events in that flush are dropped
   (`buffered.rs:163-165`) — one row poisons the batch.

**Fix:**
1. Retry transient sink failures with bounded, jittered backoff before discarding;
   keep the buffer contents on failure (clear only after sink success). Emit a
   dropped-events counter (tracing metrics) for whatever is ultimately discarded.
   Cap buffer growth during retries (backpressure: stop draining the channel while
   over a high-water mark — the channel then exerts pressure on ingest; document
   that 202 remains at-most-once unless/until a WAL lands, and leave the WAL as a
   recorded option, not implemented here).
2. Resolve the nullability mismatch at **convert time**: validate the parsed
   timestamp is set and reject the event (4xx or logged-and-counted skip) instead of
   letting it reach the batch; alternatively make the column nullable — pick the one
   consistent with how the read path treats `event_time` (it assumes presence), i.e.
   prefer convert-time rejection. Either way: one bad row must never void a flush.

### Minors

1. **Multi-sink divergence** — `buffered.rs:156-161`: fail-soft per sink with no
   replay means Delta and Iceberg copies drift permanently after a single-sink
   failure. With per-sink retry from the main fix, also make per-sink success/failure
   tracking explicit (only clear a sink's pending slice on that sink's success), or
   document single-sink deployment as the supported mode.
2. **runId not validated as UUID at ingest** —
   `crates/lineage-service/src/ingest/converter.rs:88-132`: the spec types
   `run.runId` as UUID; the converter accepts anything. Validate and reject (or
   flag-and-count) non-UUID runIds.
3. **Dead column** — `schema.rs:29` `facets_json` is always `append_null`ed
   (`schema.rs:525, 574, 623`). Decide: populate it (the read API wants facets — see
   S13's missing-endpoints work) or drop the column. Populating raw facet JSON is
   the more useful choice if cheap.
4. **`create_empty_table` misnomer** —
   `crates/lineage-service/src/writer/delta.rs:74-81` builds an unloaded handle;
   creation actually happens implicitly on first write (`delta.rs:58-72`
   open-per-append). Rename/document, and state the single-replica-writer assumption
   somewhere visible (README or module doc) — concurrent replicas would rely on
   delta-rs conflict handling that the flush layer (pre-fix) turned into data loss.
5. **Unauthenticated ingest with permissive CORS, no body-size limit** —
   `src/http.rs:38-50`. Fine for the demo stack; add a body-size limit now (cheap)
   and a config note that the surface must be gated before any shared deployment.
6. **No container healthcheck** — `environments/services/lineage-service.yaml`
   (distroless, no shell; `marquez-web` only waits for `service_started`). If the
   service grows a `--healthcheck` self-probe flag trivially, add it + compose
   healthcheck; otherwise document the gap.

## Constraints

- The ingest hot path must stay non-blocking for producers except via the documented
  backpressure mechanism.
- Crates are unpublished: change APIs freely, no compatibility shims.
- Stage your changes and propose a commit message, but do **not** run `git commit` —
  the user signs commits. Attribute AI work as "AI assisted by Isaac" if attribution
  is included.

## Verification

- `cargo test -p lineage-service` with new tests: sink failure → buffer retained and
  retried → success clears; permanent failure → counted drop; event with unset
  timestamp rejected at convert, batch unaffected; non-UUID runId rejected;
  oversized body rejected.
- `cargo clippy -p lineage-service` clean.
