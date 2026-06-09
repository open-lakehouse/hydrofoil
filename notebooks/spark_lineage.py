# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "delta-spark==4.0.1",
#     "pyspark==4.0.1",
#     "requests",
#     "marimo",
# ]
# ///

# Spark + OpenLineage + Spark Declarative Pipelines, emitting to OUR lineage service.
#
# Two parts:
#   1. A plain Spark session wired with the OpenLineage Spark listener
#      (io.openlineage:openlineage-spark) pointed at our lineage-service over HTTP.
#      It writes a UC-managed Delta table on real S3 and a derived table, so there
#      is a real input -> output lineage edge to observe.
#   2. A Spark Declarative Pipelines (SDP) run that ALSO tries to emit lineage via
#      the same listener. Whether the listener fires under `spark-pipelines run`
#      (which orchestrates its own dataflow graph) is unverified upstream — this
#      section is a spike that records the empirical outcome.
#
# Lineage target: our Rust lineage-service (NOT Marquez). It speaks OpenLineage 2.0.2
# at POST /api/v1/lineage and writes events to a Delta "events" table under
# ./.data/lineage (see environments/services/lineage-service.yaml).
#
# Maven packages (openlineage-spark, the UC connector, hadoop-aws) resolve through
# the Databricks Maven proxy via Ivy — the marimo image bakes an ivysettings.xml and
# exports SPARK_IVY_SETTINGS; on the host we fall back to spark.jars.repositories.
#
# Prerequisites:
#   - lineage-service running and reachable (docker: lineage-service:8091; host: localhost:8091).
#   - UC running with the live AWS config, bucket wired, server.managed-table.enabled=true.
#
# Run on the host:
#   uvx --directory notebooks/ marimo edit --sandbox spark_lineage.py

import marimo

__generated_with = "0.23.8"
app = marimo.App(width="medium")


@app.cell
def _():
    import marimo as mo

    return (mo,)


@app.cell
def _(mo):
    mo.md(
        """
        # Spark → OpenLineage → our lineage service

        Spark emits lineage through the standard **`openlineage-spark`** listener,
        which we point at **our own lineage service** (not Marquez). The listener
        maps each Spark job to an OpenLineage run and POSTs spec-compliant events to
        `/api/v1/lineage`, where the service buffers them into a Delta `events` table.

        We then attempt the same for a **Spark Declarative Pipelines** run — see the
        spike at the bottom.
        """
    )
    return


@app.cell
def _():
    import os

    # Catalog / table coordinates (mirror notebooks/uc_managed.py).
    UC_URI = "http://unity-catalog:8081"
    CATALOG = "demo"
    SCHEMA = "lineage_demo"
    TABLE = "events"
    DERIVED = "events_by_kind"  # derived table -> gives us an input->output edge
    # Managed-storage root for the catalog — must live under the bucket UC has
    # configured (s3.bucketPath.0=s3://olai-demo-1). UC derives each table's path.
    STORAGE_ROOT = "s3://olai-demo-1/managed"
    AWS_REGION = "eu-central-1"

    # Lineage service: docker DNS in-container, localhost on the host.
    LINEAGE_URL = os.environ.get("LINEAGE_URL", "http://lineage-service:8091")
    LINEAGE_NAMESPACE = "spark-demo"

    # Maven proxy: the image bakes ivysettings.xml and exports SPARK_IVY_SETTINGS.
    # On the host (no env var) we fall back to a plain repositories override.
    IVY_SETTINGS = os.environ.get("SPARK_IVY_SETTINGS")
    MAVEN_PROXY = "https://maven-proxy.cloud.databricks.com"
    return (
        AWS_REGION,
        CATALOG,
        DERIVED,
        IVY_SETTINGS,
        LINEAGE_NAMESPACE,
        LINEAGE_URL,
        MAVEN_PROXY,
        SCHEMA,
        STORAGE_ROOT,
        TABLE,
        UC_URI,
    )


@app.cell
def _(CATALOG, SCHEMA, STORAGE_ROOT, UC_URI):
    # Create catalog (WITH a storage_root so managed tables have somewhere to live) +
    # schema, via the UC REST API. UCSingleCatalog has no CREATE CATALOG/SCHEMA in
    # Spark SQL. Idempotent: ALREADY_EXISTS is fine on re-run.
    import requests

    base = f"{UC_URI}/api/2.1/unity-catalog"

    def _create(path, payload):
        r = requests.post(f"{base}/{path}", json=payload)
        if r.status_code not in (200, 201) and "ALREADY_EXISTS" not in r.text:
            r.raise_for_status()
        return r

    _create("catalogs", {"name": CATALOG, "comment": "lineage demo", "storage_root": STORAGE_ROOT})
    _create("schemas", {"name": SCHEMA, "catalog_name": CATALOG})

    cat = requests.get(f"{base}/catalogs/{CATALOG}").json()
    print("catalog:", cat["name"], "| storage_root:", cat.get("storage_root"))
    return


@app.cell
def _(mo):
    mo.md(
        """
        ## Spark session with the OpenLineage listener

        The session config layers two things onto the usual Delta + Unity Catalog
        setup:

        - **`spark.extraListeners`** registers `OpenLineageSparkListener`.
        - **`spark.openlineage.transport.*`** points it at our lineage service
          (`http` transport, URL + `/api/v1/lineage` endpoint, a namespace).
        - **`spark.openlineage.columnLineage.datasetLineageEnabled=true`** opts into
          column-level lineage facets.

        `openlineage-spark_2.13` is a Maven package, resolved through the Databricks
        Maven proxy via Ivy.
        """
    )
    return


@app.cell
def _(
    AWS_REGION,
    CATALOG,
    IVY_SETTINGS,
    LINEAGE_NAMESPACE,
    LINEAGE_URL,
    MAVEN_PROXY,
    UC_URI,
):
    from delta import configure_spark_with_delta_pip
    import pyspark

    builder = (
        pyspark.sql.SparkSession.builder.appName("spark-lineage")
        .config("spark.sql.extensions", "io.delta.sql.DeltaSparkSessionExtension")
        .config("spark.sql.catalog.spark_catalog", "io.unitycatalog.spark.UCSingleCatalog")
        .config(f"spark.sql.catalog.{CATALOG}", "io.unitycatalog.spark.UCSingleCatalog")
        .config(f"spark.sql.catalog.{CATALOG}.uri", UC_URI)
        .config(f"spark.sql.catalog.{CATALOG}.token", "")  # UC OSS dev: no auth
        .config("spark.sql.defaultCatalog", CATALOG)
        # S3A region for the real AWS bucket. We do NOT set access/secret keys or a
        # credentials provider here: UC vends STS temporary credentials and the
        # UCSingleCatalog connector injects them into the s3a client per table.
        .config("spark.hadoop.fs.s3.impl", "org.apache.hadoop.fs.s3a.S3AFileSystem")
        .config("spark.hadoop.fs.s3a.endpoint.region", AWS_REGION)
        # --- OpenLineage listener -> our lineage service (NOT Marquez) ---
        .config("spark.extraListeners", "io.openlineage.spark.agent.OpenLineageSparkListener")
        .config("spark.openlineage.transport.type", "http")
        .config("spark.openlineage.transport.url", LINEAGE_URL)
        .config("spark.openlineage.transport.endpoint", "/api/v1/lineage")
        .config("spark.openlineage.namespace", LINEAGE_NAMESPACE)
        .config("spark.openlineage.columnLineage.datasetLineageEnabled", "true")
    )

    # Maven package resolution through the Databricks proxy. In-container the image
    # bakes an ivysettings.xml (sole resolver = the proxy) and exports
    # SPARK_IVY_SETTINGS; on the host we just add the proxy as a repository.
    if IVY_SETTINGS:
        builder = builder.config("spark.jars.ivySettings", IVY_SETTINGS)
    else:
        builder = builder.config("spark.jars.repositories", MAVEN_PROXY)

    extra_packages = [
        "io.unitycatalog:unitycatalog-spark_2.13:0.4.0",
        "org.apache.hadoop:hadoop-aws:3.4.0",
        # Use the _2.13 coordinate at 1.47.1: it's the latest mirrored on the
        # Databricks Maven proxy AND is Spark-4.0-compatible. The bare
        # `openlineage-spark` coordinate only goes to 1.8.0 on the proxy, which
        # breaks Spark 4.0 ("No active or default Spark session found").
        "io.openlineage:openlineage-spark_2.13:1.47.1",
    ]

    spark = configure_spark_with_delta_pip(builder, extra_packages=extra_packages).getOrCreate()
    spark.sparkContext.setLogLevel("WARN")
    return (spark,)


@app.cell
def _(CATALOG, SCHEMA, TABLE, spark):
    # Source: a MANAGED Delta table (no LOCATION). UC assigns the storage path and
    # vends creds; the catalogManaged feature flag declares the UC managed contract.
    spark.sql(
        f"""
        CREATE TABLE IF NOT EXISTS {CATALOG}.{SCHEMA}.{TABLE} (
            id BIGINT, kind STRING, ts TIMESTAMP
        ) USING DELTA
        TBLPROPERTIES ('delta.feature.catalogManaged' = 'supported')
        """
    )
    spark.sql(
        f"""
        INSERT INTO {CATALOG}.{SCHEMA}.{TABLE} VALUES
          (1, 'login',  TIMESTAMP '2026-06-02 09:00:00'),
          (2, 'click',  TIMESTAMP '2026-06-02 09:01:00'),
          (3, 'click',  TIMESTAMP '2026-06-02 09:02:00'),
          (4, 'logout', TIMESTAMP '2026-06-02 09:05:00')
        """
    )
    return


@app.cell
def _(CATALOG, DERIVED, SCHEMA, TABLE, spark):
    # Derived table: this CTAS reads {TABLE} and writes {DERIVED}, producing the
    # input -> output edge (and column-level lineage) the listener reports.
    spark.sql(f"DROP TABLE IF EXISTS {CATALOG}.{SCHEMA}.{DERIVED}")
    spark.sql(
        f"""
        CREATE TABLE {CATALOG}.{SCHEMA}.{DERIVED}
        USING DELTA
        TBLPROPERTIES ('delta.feature.catalogManaged' = 'supported')
        AS
        SELECT kind, COUNT(*) AS n, MAX(ts) AS last_seen
        FROM {CATALOG}.{SCHEMA}.{TABLE}
        GROUP BY kind
        """
    )
    spark.table(f"{CATALOG}.{SCHEMA}.{DERIVED}").show(truncate=False)
    return


@app.cell
def _(LINEAGE_URL, mo):
    # Health check + where to look. The lineage service buffers events into a Delta
    # "events" table. With the live Docker stack it lands at
    # `environments/.data/lineage/events` on the host (compose project dir is
    # environments/); inspect it with deltalake/DuckDB, e.g.:
    #   duckdb -c "INSTALL delta; LOAD delta;
    #     SELECT job_namespace, job_name, event_type
    #     FROM delta_scan('environments/.data/lineage/events')
    #     WHERE job_namespace='spark-demo' ORDER BY event_time"
    import requests as _rq

    try:
        health = _rq.get(f"{LINEAGE_URL}/health", timeout=5).text
    except Exception as e:  # noqa: BLE001
        health = f"unreachable: {e}"

    mo.md(
        f"""
        **lineage-service** `{LINEAGE_URL}/health` → `{health}`

        **Verified 2026-06-08:** the workload above emits OpenLineage run events
        (parent `spark_lineage` START/COMPLETE plus per-statement create/insert/CTAS
        runs) to the service under namespace `spark-demo`, written to the Delta
        `events` table.

        Caveat: for UC catalog-managed Delta tables on Spark 4.0, the listener emits
        the runs but the **dataset `inputs`/`outputs` and `columnLineage` facets are
        often empty** ("Could not extract dataset identifier"). Plain file/parquet
        jobs produce richer dataset lineage.
        """
    )
    return


@app.cell
def _(mo):
    mo.md(
        """
        ## Spike: Spark Declarative Pipelines + OpenLineage

        SDP defines a pipeline as a graph of materialized views / streaming tables
        (see `pipelines/transformations/pipeline.py`) and runs it with the external
        **`spark-pipelines`** CLI. Because that CLI is built on `spark-submit`, the
        OpenLineage listener and transport are configured in the pipeline spec's
        `configuration:` block (`pipelines/spark-pipeline.yml`).

        **Requires pyspark 4.1.0+.** Verified 2026-06-08: SDP (the `pyspark.pipelines`
        module and the `spark-pipelines` launcher) is **absent from pyspark 4.0.1** —
        which is what this notebook/the marimo image pins — and only appears in 4.1.0.
        So this section needs a 4.1.0 runtime; under 4.0.1 the import of
        `pyspark.pipelines` in the transformation file fails.

        **Unverified upstream:** even on 4.1.0, SDP orchestrates its own dataflow
        graph, so it's not documented whether the OpenLineage `SparkListener` fires
        per flow. Run it on a 4.1.0 runtime and record what lands under namespace
        `sdp-demo` in the lineage service.
        """
    )
    return


@app.cell
def _(MAVEN_PROXY):
    # Run the SDP pipeline. `spark-pipelines` ships with pyspark **4.1.0+** (NOT 4.0.1
    # — see the note above). Packages are passed at run time (same coordinates as the
    # session above); the spec carries the UC + S3A + OpenLineage config. We pass the
    # Maven proxy as a repository so resolution works in-network.
    import subprocess
    from pathlib import Path

    packages = ",".join(
        [
            "io.delta:delta-spark_2.13:4.0.1",
            "io.unitycatalog:unitycatalog-spark_2.13:0.4.0",
            "org.apache.hadoop:hadoop-aws:3.4.0",
            # 1.47.1 is the latest on the Databricks Maven proxy and Spark-4-compatible.
            "io.openlineage:openlineage-spark_2.13:1.47.1",
        ]
    )
    spec = Path("pipelines/spark-pipeline.yml")

    proc = subprocess.run(
        [
            "spark-pipelines",
            "run",
            "--spec",
            str(spec),
            "--conf",
            f"spark.jars.packages={packages}",
            "--conf",
            f"spark.jars.repositories={MAVEN_PROXY}",
        ],
        capture_output=True,
        text=True,
    )
    print("returncode:", proc.returncode)
    print(proc.stdout[-4000:])
    print(proc.stderr[-4000:])
    return


@app.cell
def _(mo):
    mo.md(
        """
        ### Spike result (2026-06-08)

        **Plain-Spark leg: ✅ verified.** The Spark session above (openlineage-spark
        `_2.13:1.47.1`) emits OpenLineage run events to our lineage service — confirmed
        by 22 events under namespaces `spark-demo`/`spark-minimal` landing in the
        service's Delta `events` table (parent application run + per-statement
        create/insert/CTAS runs). Caveat: for UC catalog-managed Delta tables the
        dataset `inputs`/`outputs`/`columnLineage` facets come back empty; plain
        parquet jobs carry richer dataset lineage.

        **SDP leg: ⏸ blocked on runtime version.** SDP (`pyspark.pipelines` +
        `spark-pipelines`) is **not in pyspark 4.0.1** (this notebook's pin) — it
        first appears in **4.1.0**. So `spark-pipelines run` can't execute here until
        the runtime is bumped to 4.1.0. Whether the OpenLineage listener fires under
        SDP's dataflow-graph executor remains the open question, to be answered once
        a 4.1.0 runtime is available. Tracked in `docs/content/content-map.md`
        (Gaps #7/#13).
        """
    )
    return


if __name__ == "__main__":
    app.run()
