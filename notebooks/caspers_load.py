# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "delta-spark==4.1.0",
#     "pyspark==4.1.2",
#     "polars",
#     "numpy",
#     "pyarrow",
#     "requests",
#     "adbc-driver-flightsql>=1.9.0",
#     "marimo",
# ]
# ///

# Load Casper's Ghost Kitchen demo data into UC-managed Delta on real AWS S3.
#
# This is the one-off loader behind the stage notebooks. It:
#   1. generates the whole marketplace deterministically (caspers_gen.generate_all),
#   2. creates the `caspers` catalog + bronze/silver/gold/ml schemas via the UC REST
#      API (UCSingleCatalog has no CREATE CATALOG/SCHEMA in Spark SQL), and
#   3. writes every frame as a UC-MANAGED Delta table (no LOCATION; the catalogManaged
#      feature flag declared) on the catalog's storage_root — the pattern proven in
#      notebooks/uc_managed.py.
#
# Every Spark write runs with the OpenLineage listener wired to our lineage service
# (namespace `caspers-load`), so the loaded tables become lineage dataset entities
# you can find in the lineage graph / Marquez UI (see notebooks/spark_lineage.py).
#
# Spark jars (UC 0.5 connector from branch-0.5, delta-spark, openlineage-spark,
# hadoop-aws) are baked onto the classpath in the marimo image — no runtime Ivy.
#
# Prerequisites:
#   - UC running with the live AWS config (just env-local-up): bucket wired via
#     s3.bucketPath.0 / s3.awsRoleArn.0 and server.managed-table.enabled=true.
#   - UC reachable at localhost:8081; lineage-service reachable (optional but
#     recommended so lineage lands).
#
# Run on the host (from the repo so caspers_gen.py is importable):
#   uvx --directory notebooks/ marimo edit --sandbox caspers_load.py

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
        # Load Casper's Ghost Kitchen → UC-managed Delta

        Generates the deterministic demo dataset and writes it as **UC-managed Delta
        tables** across four schemas — `caspers.bronze / silver / gold / ml` — on the
        real S3 bucket UC manages. Each write emits **OpenLineage** to our lineage
        service (namespace `caspers-load`) so the tables show up in the lineage graph.
        """
    )
    return


@app.cell
def _():
    import os

    UC_URI = os.environ.get("UC_URI", "http://localhost:8081")
    CATALOG = "caspers"
    SCHEMAS = ["bronze", "silver", "gold", "ml"]
    # Managed-storage root — must live under the bucket UC has configured
    # (s3.bucketPath.0=s3://olai-demo-1). UC derives each table's path beneath this.
    STORAGE_ROOT = "s3://olai-demo-1/managed"
    AWS_REGION = "eu-central-1"

    # Lineage service: docker DNS in-container, localhost on the host.
    LINEAGE_URL = os.environ.get("LINEAGE_URL", "http://lineage-service:8091")
    LINEAGE_NAMESPACE = "caspers-load"

    SEED = 42

    # Hydrofoil bulk-ingest stretch (read here so `os` is imported in one cell only).
    TRY_HYDROFOIL_WRITE = os.environ.get("TRY_HYDROFOIL_WRITE") == "1"
    HYDROFOIL_ENDPOINT = os.environ.get("HYDROFOIL_ENDPOINT", "grpc://localhost:50052")
    return (
        AWS_REGION,
        CATALOG,
        HYDROFOIL_ENDPOINT,
        LINEAGE_NAMESPACE,
        LINEAGE_URL,
        SCHEMAS,
        SEED,
        STORAGE_ROOT,
        TRY_HYDROFOIL_WRITE,
        UC_URI,
    )


@app.cell
def _(SEED, mo):
    # Generate the whole dataset up front (engine-agnostic polars frames).
    import caspers_gen

    frames = caspers_gen.generate_all(seed=SEED)
    mo.ui.table(caspers_gen.table_summary(frames).to_dicts(), label="Generated tables")
    return caspers_gen, frames


@app.cell
def _(CATALOG, SCHEMAS, STORAGE_ROOT, UC_URI):
    # Create catalog (WITH a storage_root) + schemas via the UC REST API. Idempotent:
    # ALREADY_EXISTS is fine on re-run.
    import requests

    base = f"{UC_URI}/api/2.1/unity-catalog"

    def _create(path, payload):
        r = requests.post(f"{base}/{path}", json=payload)
        if r.status_code not in (200, 201) and "ALREADY_EXISTS" not in r.text:
            r.raise_for_status()
        return r

    _create("catalogs", {"name": CATALOG, "comment": "Casper's Ghost Kitchen demo", "storage_root": STORAGE_ROOT})
    for _schema in SCHEMAS:
        _create("schemas", {"name": _schema, "catalog_name": CATALOG})

    _cat = requests.get(f"{base}/catalogs/{CATALOG}").json()
    print("catalog:", _cat["name"], "| storage_root:", _cat.get("storage_root"))
    print("schemas:", [s["name"] for s in requests.get(f"{base}/schemas", params={"catalog_name": CATALOG}).json()["schemas"]])
    return


@app.cell
def _(AWS_REGION, CATALOG, LINEAGE_NAMESPACE, LINEAGE_URL, UC_URI):
    import pyspark

    # UCSingleCatalog + OpenLineage listener. Jars are baked into the marimo image;
    # UC vends STS temp creds per table (no access/secret keys here). See
    # notebooks/uc_managed.py + notebooks/spark_lineage.py.
    spark = (
        pyspark.sql.SparkSession.builder.appName("caspers-load")
        .config("spark.sql.extensions", "io.delta.sql.DeltaSparkSessionExtension")
        .config("spark.sql.catalog.spark_catalog", "io.unitycatalog.spark.UCSingleCatalog")
        .config(f"spark.sql.catalog.{CATALOG}", "io.unitycatalog.spark.UCSingleCatalog")
        .config(f"spark.sql.catalog.{CATALOG}.uri", UC_URI)
        .config(f"spark.sql.catalog.{CATALOG}.token", "")
        .config("spark.sql.defaultCatalog", CATALOG)
        .config("spark.hadoop.fs.s3.impl", "org.apache.hadoop.fs.s3a.S3AFileSystem")
        .config("spark.hadoop.fs.s3a.endpoint.region", AWS_REGION)
        # OpenLineage listener -> our lineage service so the loaded tables become entities.
        .config("spark.extraListeners", "io.openlineage.spark.agent.OpenLineageSparkListener")
        .config("spark.openlineage.transport.type", "http")
        .config("spark.openlineage.transport.url", LINEAGE_URL)
        .config("spark.openlineage.transport.endpoint", "/api/v1/lineage")
        .config("spark.openlineage.namespace", LINEAGE_NAMESPACE)
        .config("spark.openlineage.columnLineage.datasetLineageEnabled", "true")
        .config("spark.sql.session.timeZone", "UTC")
        .getOrCreate()
    )
    spark.sparkContext.setLogLevel("WARN")
    return (spark,)


@app.cell
def _(mo):
    mo.md(
        """
        ## Write each frame as a UC-managed Delta table

        We hand Spark each polars frame via Arrow, then `CREATE TABLE … USING DELTA
        TBLPROPERTIES ('delta.feature.catalogManaged' = 'supported')` (no LOCATION =
        managed) and `INSERT`/overwrite the rows. Re-running is idempotent (seeded data
        + overwrite). Decimal/timestamp types are preserved so the masking and
        classification stories downstream have correct types to work with.
        """
    )
    return


@app.cell
def _(frames, spark):
    import polars as pl
    import pyspark.sql.functions as F  # noqa: F401 (handy in interactive debugging)
    from pyspark.sql.types import (
        BooleanType, DoubleType, LongType, StringType, StructField, StructType, TimestampType,
    )

    # polars dtype -> Spark type. Integers land as LongType, floats as DoubleType.
    # (We keep money as DoubleType for portability rather than DECIMAL — the connector's
    # Arrow path is happiest with primitive types; the visualizations don't need exact
    # decimal. If exact decimal is wanted later, cast per-column in SQL after load.)
    def _spark_field(name, dtype):
        if dtype in (pl.Int8, pl.Int16, pl.Int32, pl.Int64, pl.UInt8, pl.UInt16, pl.UInt32, pl.UInt64):
            t = LongType()
        elif dtype in (pl.Float32, pl.Float64):
            t = DoubleType()
        elif dtype == pl.Boolean:
            t = BooleanType()
        elif dtype in (pl.Datetime, pl.Date):
            t = TimestampType()
        else:
            t = StringType()
        return StructField(name, t, True)

    def _to_spark(df_pl):
        # Cast dates to datetimes so Spark sees TIMESTAMP uniformly.
        df_pl = df_pl.with_columns(
            [pl.col(c).cast(pl.Datetime("us")) for c, d in df_pl.schema.items() if d == pl.Date]
        )
        schema = StructType([_spark_field(c, d) for c, d in df_pl.schema.items()])
        return spark.createDataFrame(df_pl.to_pandas(), schema=schema)

    return (_to_spark,)


@app.cell
def _(_to_spark, frames, spark):
    # Create + overwrite every managed table. fqname is `caspers.<schema>.<table>`.
    written = []
    for _fq, _pl in frames.items():
        _sdf = _to_spark(_pl)
        # Temp view -> CTAS-with-feature-flag keeps it managed and avoids hand-writing DDL
        # for ~20 schemas. We CREATE OR REPLACE so re-runs are clean.
        _view = "_load_" + _fq.replace(".", "_")
        _sdf.createOrReplaceTempView(_view)
        spark.sql(f"DROP TABLE IF EXISTS {_fq}")
        spark.sql(
            f"CREATE TABLE {_fq} USING DELTA "
            f"TBLPROPERTIES ('delta.feature.catalogManaged' = 'supported') "
            f"AS SELECT * FROM {_view}"
        )
        written.append((_fq, _sdf.count()))
        print("wrote", _fq, written[-1][1], "rows")
    return (written,)


@app.cell
def _(mo, written):
    mo.ui.table(
        [{"table": fq, "rows": n} for fq, n in written],
        label="Written UC-managed Delta tables",
    )
    return


@app.cell
def _(CATALOG, spark):
    # Confirm one table is MANAGED and see its UC-assigned S3 location.
    spark.sql(f"DESCRIBE EXTENDED {CATALOG}.bronze.orders").show(truncate=False)
    return


@app.cell
def _(mo):
    mo.md(
        """
        ## (Stretch, unverified) Write via Hydrofoil bulk-ingest

        Hydrofoil is our Flight SQL server. The ADBC Flight SQL driver exposes a
        **bulk-ingest** operation (`cursor.adbc_ingest(table, arrow_data, mode=...)`)
        that streams Arrow record batches straight to the server — the right route for
        a write through Hydrofoil (no `INSERT … SELECT` SQL), and a clean fit since the
        generator already produces Arrow.

        **This path is unverified** — Hydrofoil hasn't been exercised for writes. The
        cell below is wrapped so a failure is *reported, not raised*, in the same spirit
        as the `duckdb_flight.py` / `uc_duckdb.py` caveats. Toggle `TRY_HYDROFOIL_WRITE`
        to attempt it.
        """
    )
    return


@app.cell
def _(HYDROFOIL_ENDPOINT, TRY_HYDROFOIL_WRITE, frames, mo):
    _out = "Skipped (set `TRY_HYDROFOIL_WRITE=1` to attempt the bulk-ingest write)."
    if TRY_HYDROFOIL_WRITE:
        try:
            from adbc_driver_flightsql.dbapi import connect

            import _demo_auth

            # A small frame to probe the bulk-ingest path.
            probe_fq = "caspers.bronze.vendors"
            arrow_tbl = frames[probe_fq].to_arrow()
            target = probe_fq.split(".")[-1] + "_hf_probe"

            with connect(
                HYDROFOIL_ENDPOINT,
                db_kwargs=_demo_auth.db_kwargs(
                    "alice@example.com",
                    extra={
                        "x-openlineage-job-namespace": "caspers-load",
                        "x-openlineage-job-name": "hydrofoil_bulk_ingest_probe",
                    },
                ),
            ) as conn:
                cur = conn.cursor()
                n = cur.adbc_ingest(target, arrow_tbl, mode="create")
                cur.close()
            _out = f"✅ Hydrofoil bulk-ingest wrote {n} rows to `{target}`."
        except Exception as e:  # noqa: BLE001 — record the empirical outcome
            _out = f"❌ Hydrofoil bulk-ingest not supported / failed:\n\n```\n{e}\n```"

    mo.md(_out)
    return


if __name__ == "__main__":
    app.run()
