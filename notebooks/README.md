# Notebooks

[marimo](https://marimo.io) notebooks demonstrating the Open Lakehouse stack. Each
is a standalone PEP-723 script (deps declared in the header) and runs either inside
the marimo container (`just env-up`, editor at Envoy `:10130`) or on the host:

```bash
uvx --directory notebooks/ marimo edit --sandbox <notebook>.py
```

### Demo story (Casper's Ghost Kitchen)

A five-stage narrative; each stage is an app-mode dashboard (`marimo run`). All
stages read through `_caspers_read.py` (Spark over UC Delta *or* hydrofoil Flight
SQL, chosen by `CASPERS_BACKEND`) and fall back to `caspers_gen.py`'s deterministic
seeded data, so they render offline.

| Notebook | Shows |
|---|---|
| `stage1_marketplace.py` | Marketplace overview: GMV, contribution margin, vendor performance |
| `stage2_governance.py` | **Cedar** governance over Flight SQL: row filters + column masks per principal |
| `stage3_metric_views.py` | UC **metric views** as the single source of truth for enterprise metrics |
| `stage4_lineage_classification.py` | **Column-level lineage** + PII classification propagation |
| `stage5_predict_act.py` | Forecast-vs-actual, demand drivers, autonomous agent actions |

### Lineage / cross-engine deep-dives

| Notebook | Shows | Notes |
|---|---|---|
| `spark_lineage.py` | Spark emits **OpenLineage** to our lineage service | see below |
| `lineage_metadata.py` | Client-forwarded **OpenLineage metadata** (job name/tags/owners, parent run, agent context) as Flight SQL call headers | ADR 0012; host hydrofoil on `:50052` |
| `column_lineage.py` | **Column-level lineage**: `INSERT … SELECT` writes through hydrofoil, field-level graph read back from the lineage service | facet on outputs; host hydrofoil on `:50052` |

### Shared modules

`caspers_gen.py` (deterministic data generator), `_caspers_read.py` (backend-agnostic
SQL → polars), `_demo_auth.py` (forwards the Cedar principal + UC token to hydrofoil as
Flight SQL call headers — also imported by the loaders in `../notebooks_local/`).

## `spark_lineage.py`

Spark emits lineage through the standard `io.openlineage:openlineage-spark` listener
pointed at **our own lineage service** (`/api/v1/lineage`, port `8091`) — not Marquez.
It runs a UC-managed Delta workload (a source table and a derived CTAS) using
`io.openlineage:openlineage-spark_2.13:1.47.1`. **✅ Verified 2026-06-08:** run events
land in the lineage service's Delta `events` table (`environments/.data/lineage/events`).
Caveat: UC catalog-managed Delta tables emit runs but **sparse dataset/columnLineage
facets** on Spark 4.0; plain parquet jobs carry richer lineage.

Prerequisites: the lineage service must be running (docker DNS `lineage-service:8091`,
or `localhost:8091` on the host); UC configured for the real S3 bucket; the `demo`
catalog created via the UC REST API.

**Maven proxy.** Spark resolves packages (the UC connector, `hadoop-aws`,
`openlineage-spark`) through the Databricks Maven proxy. The marimo image bakes an
`ivysettings.xml` and exports `SPARK_IVY_SETTINGS`; on the host the notebook falls
back to `spark.jars.repositories=https://maven-proxy.cloud.databricks.com`.
