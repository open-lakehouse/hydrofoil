# lineage-service

An OpenLineage HTTP ingest service. It accepts [OpenLineage](https://openlineage.io)
events over HTTP, buffers them asynchronously, and writes them to a lakehouse **events
table** (Delta Lake today; Apache Iceberg behind a feature flag). It is the Rust
successor to the original Go lineage ingest service.

## What it does

```
                  POST /api/v1/lineage[/batch]
                              │
                              ▼
   ┌──────────┐   convert    ┌───────────────┐   flush    ┌────────────┐   append   ┌────────────┐
   │  http.rs │ ───────────▶ │ ingest/       │ ─────────▶ │ writer/    │ ─────────▶ │ TableSink  │
   │ (axum)   │   JSON→proto │  converter.rs │  buffered  │  buffered  │  RecordBatch│ (delta/…)  │
   └──────────┘              └───────────────┘            └────────────┘            └────────────┘
        │ 202 Accepted (does not block on the write)
        ▼
```

1. **Ingest** (`src/http.rs`) — `POST /api/v1/lineage` (one event) and
   `POST /api/v1/lineage/batch` (a JSON array). Handlers parse + enqueue, then return
   `202 Accepted`; they never block on the lakehouse write. A `GET /health` liveness probe
   is also mounted.
2. **Convert** (`src/ingest/converter.rs`) — classifies each event as a Run / Job / Dataset
   event, validates `eventTime`, lifts the nested `columnLineage` facet into a typed field,
   and preserves the original wire bytes in `raw_json`. Events are held as zero-copy
   [`buffa`](https://crates.io/crates/buffa) views over owned bytes.
3. **Buffer** (`src/writer/buffered.rs`) — a background tokio task batches events and flushes
   on whichever comes first: a size threshold (`BUFFER_SIZE`) or a time interval
   (`FLUSH_INTERVAL_MS`). `enqueue` applies **backpressure** when the bounded channel is full
   (it does not drop events). On shutdown the channel drains before exit.
4. **Sink** (`src/writer/sink.rs`) — each flushed `RecordBatch` is fanned out to one or more
   `TableSink`s. A sink failure is logged and the remaining sinks still run (fail-soft).

### The events table schema

Every event is flattened into a 15-column Arrow `RecordBatch` (`src/writer/schema.rs`):
`event_kind`, `event_type`, `event_time` (`Timestamp(µs, UTC)`), `producer`, `schema_url`,
`run_id`, `job_namespace`, `job_name`, `dataset_namespace`, `dataset_name`, `facets_json`,
`inputs_json`, `outputs_json`, `column_lineage_json`, and `raw_json`. The `*_json` columns
carry structured detail as JSON (column lineage mirrors the OpenLineage 1.2.0 shape), and
`raw_json` keeps the original event so no information is lost in the flattening.

## Running

```sh
cargo run -p lineage-service
# then, from another shell:
curl -XPOST localhost:8091/api/v1/lineage \
  -H 'content-type: application/json' \
  --data-binary @crates/lineage-service/examples/lineage/single/run-event.json
curl localhost:8091/health    # -> OK
```

By default it writes a local Delta table at `/data/events`. Point `DELTA_TABLE_PATH` at a
writable directory (or an `s3://…` URI) to change that.

### Configuration (`src/config.rs`)

All configuration is via environment variables. An **unset** variable falls back to the
documented default; a variable that is **set but unparsable** is a hard error so a
misconfigured deployment refuses to start rather than silently running on defaults.

| Variable | Default | Meaning |
|----------|---------|---------|
| `LINEAGE_SERVICE_PORT` | `8091` | HTTP listen port |
| `TABLE_SINKS` | `delta` | Comma-separated sinks: `delta`, `iceberg` (order preserved; unknown values are rejected) |
| `DELTA_TABLE_PATH` | `/data/events` | Events-table location — a bare path or any object-store URI (`s3://…`, `file://…`) |
| `DELTA_PARTITION_COLS` | `event_kind` | Comma-separated partition columns (empty for unpartitioned) |
| `AWS_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_ENDPOINT_URL`, `AWS_S3_ALLOW_UNSAFE_RENAME` | — | Forwarded to the object store when writing to S3-compatible storage |
| `BUFFER_SIZE` | `100` | Flush once this many events are buffered |
| `FLUSH_INTERVAL_MS` | `500` | Flush at least this often, even below `BUFFER_SIZE` |
| `CHANNEL_CAPACITY` | `1000` | Bounded ingest channel depth (backpressure point) |

`RUST_LOG` controls tracing verbosity (e.g. `RUST_LOG=lineage_service=info`).

## Building with the Iceberg sink

The Apache Iceberg sink lives behind the non-default `iceberg` cargo feature:

```sh
cargo build -p lineage-service --features iceberg
```

It is **off by default** because `iceberg`/`iceberg-catalog-rest` pull `arrow`/`parquet` 57
while the Delta path runs on `arrow` 58 (via `deltalake`), and the sink's own `parquet` dep
is 55 — a three-way version skew. Keeping it off-by-default means the default build is a
single, coherent `arrow`-58 graph. Reconciling that skew (and re-validating the sink) is a
tracked follow-up; until then `--features iceberg` is not expected to build cleanly.

When enabled, the sink is configured with `ICEBERG_CATALOG_URI`, `ICEBERG_WAREHOUSE`,
`ICEBERG_NAMESPACE`, `ICEBERG_TABLE`, `ICEBERG_PARTITION_COLS`, and an optional
`ICEBERG_TOKEN` (see `iceberg_from_env` in `src/config.rs`).

## Path forward: Unity Catalog OSS integration

Today the Delta sink (`src/writer/delta.rs`) writes to a location given directly by
`DELTA_TABLE_PATH`, with credentials taken from static `AWS_*` environment variables. The
next step is to write the events table as a **Unity Catalog-managed Delta table**, resolving
its location and write credentials from UC OSS the same way the query engine
(`hydrofoil`) resolves them for reads.

The intended design (see
[ADR 0009](../../docs/adr/0009-lineage-service-unity-catalog-write-path.md)) introduces a
`TableLocator` seam that resolves `(location, object_store)` for the events table:

- **`StaticLocator`** — today's behavior: a raw URI + storage options. Keeps local and test
  setups simple.
- **`UnityLocator`** — resolves the events table by `catalog.schema.table` via
  `unitycatalog-client`, vends **write** credentials with
  `UnityObjectStoreFactory::for_table(name, TableOperation::ReadWrite)`, and injects the
  resulting credential-refreshing object store into the Delta builder
  (`DeltaTableBuilder::with_storage_backend`). The `DeltaWriter` becomes agnostic to *how*
  the location and store are obtained.

Feasibility is confirmed against the pinned `unitycatalog-rs` revision: the UC server grants
`s3:PutObject`/`s3:DeleteObject` for the `ReadWrite` operation, so write-credential vending
works end-to-end. v1 will assume the operator pre-creates the UC table (the writer fails fast
with a bootstrap hint if it is missing) and that the table uses direct commits (not the UC
commit coordinator). Implementation is deferred to a dedicated pass.

## Layout

```
src/
  http.rs              HTTP ingestion surface (axum router + handlers)
  config.rs            environment-based configuration + validation
  ingest/converter.rs  OpenLineage JSON → proto, column-lineage lifting
  writer/
    buffered.rs        async buffering + size/interval flush + backpressure
    sink.rs            TableSink trait + SinkError
    delta.rs           Delta Lake sink
    iceberg.rs         Apache Iceberg sink (feature = "iceberg")
    schema.rs          canonical Arrow schema + event serialization
  proto/lineage.v1.rs  generated by buffa — do not edit
tests/integration_test.rs   Delta round-trip + column-lineage tests
examples/lineage/           sample OpenLineage event fixtures
```
