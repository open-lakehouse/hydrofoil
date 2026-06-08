# Notebooks

[marimo](https://marimo.io) notebooks demonstrating the Open Lakehouse stack. Each
is a standalone PEP-723 script (deps declared in the header) and runs either inside
the marimo container (`just env-up`, editor at Envoy `:10130`) or on the host:

```bash
uvx --directory notebooks/ marimo edit --sandbox <notebook>.py
```

| Notebook | Shows | Notes |
|---|---|---|
| `uc_managed.py` | Spark creates a **UC-managed** Delta table on real AWS S3; UC vends STS creds | catalog-managed commits; verified path |
| `uc_crud.py` | Spark CRUD on **external** Delta tables via UC | local S3 endpoint, static keys |
| `uc_duckdb.py` | **DuckDB** reads/appends a UC-managed table (created via Spark) | cross-engine; blocked by a DuckDB Content-Type bug |
| `policy_demo.py` | **Cedar** governance over Flight SQL: row filters + column masks per principal | governance demo via hydrofoil |
| `spark_lineage.py` | Spark emits **OpenLineage** to our lineage service; **SDP** spike | see below |
| `client.py` | Minimal Flight SQL / ADBC client | |
| `duckdb_flight.py` | **DuckDB** reaches hydrofoil's Flight SQL endpoint via the `adbc_scanner` extension | `SELECT 1` connectivity check; host-only (needs network to install the community extension) |

## `spark_lineage.py` + `pipelines/`

Spark emits lineage through the standard `io.openlineage:openlineage-spark` listener
pointed at **our own lineage service** (`/api/v1/lineage`, port `8091`) — not Marquez.
The notebook has two parts:

1. **Plain Spark session** — runs a UC-managed Delta workload (a source table and a
   derived CTAS). Uses `io.openlineage:openlineage-spark_2.13:1.47.1`. **✅ Verified
   2026-06-08:** run events land in the lineage service's Delta `events` table
   (`environments/.data/lineage/events`). Caveat: UC catalog-managed Delta tables
   emit runs but **sparse dataset/columnLineage facets** on Spark 4.0; plain parquet
   jobs carry richer lineage.
2. **Spark Declarative Pipelines spike** — the sidecar `pipelines/` directory holds a
   `spark-pipeline.yml` spec and `transformations/pipeline.py` (two materialized
   views). The notebook shells out to `spark-pipelines run`. **⏸ Requires pyspark
   4.1.0+** — SDP (`pyspark.pipelines` + `spark-pipelines`) is absent from 4.0.1
   (this notebook's pin) and first appears in 4.1.0. Whether the OpenLineage listener
   fires under SDP's dataflow-graph orchestration is the open question, to be answered
   on a 4.1.0 runtime.

```
pipelines/
  spark-pipeline.yml              # SDP spec: UC + S3A + OpenLineage configuration
  transformations/pipeline.py     # @dp.materialized_view graph -> UC-managed Delta
```

Prerequisites: the lineage service must be running (docker DNS `lineage-service:8091`,
or `localhost:8091` on the host); UC configured for the real S3 bucket; the `demo`
catalog and the `sdp_demo` schema created via the UC REST API.

**Maven proxy.** Spark resolves packages (the UC connector, `hadoop-aws`,
`openlineage-spark`) through the Databricks Maven proxy. The marimo image bakes an
`ivysettings.xml` and exports `SPARK_IVY_SETTINGS`; on the host the notebook falls
back to `spark.jars.repositories=https://maven-proxy.cloud.databricks.com`.
