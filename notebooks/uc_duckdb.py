# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "duckdb>=1.5.0",
#     "delta-spark==4.0.1",
#     "pyspark==4.0.1",
#     "requests",
#     "marimo",
# ]
# ///

# Read a UC MANAGED Delta table with DuckDB, then INSERT more rows through DuckDB.
#
# DuckDB's `unity_catalog` extension (depends on `delta`) can ATTACH a Unity Catalog server and
# both READ and APPEND (INSERT) into managed Delta tables. UC vends short-lived S3 credentials per
# table and the extension wires them into DuckDB automatically — no separate S3 secret needed.
#
# Constraints of the DuckDB UC path (as of duckdb 1.5.x):
#   - INSERT (append) only. No UPDATE / DELETE / MERGE / OVERWRITE.
#   - No DDL: DuckDB cannot CREATE or DROP a UC table. The table must already exist.
# So this notebook first creates the catalog/schema (UC REST) and the managed table (Spark, the
# proven path from uc_managed.py), then hands off to DuckDB for the read + insert.
#
# Prerequisites (same live AWS setup as uc_managed.py):
#   - UC running with the live AWS config (just env-local-up): bucket wired via s3.bucketPath.0 /
#     s3.awsRoleArn.0 / s3.accessKey.0 / s3.secretKey.0, and `server.managed-table.enabled=true`.
#   - UC reachable at localhost:8081.
#
# Run on the host:
#   uvx --directory notebooks/ marimo edit --sandbox uc_duckdb.py

import marimo

__generated_with = "0.18.4"
app = marimo.App()


@app.cell
def _():
    UC_URI = "http://localhost:8081"
    # DuckDB's unity_catalog extension builds request URLs by concatenating ENDPOINT + path, so the
    # ENDPOINT must carry a scheme and a host that resolves. Use 127.0.0.1 (not localhost) to dodge
    # IPv6 (::1) resolution quirks that surface as "Could not resolve hostname".
    UC_ENDPOINT = "http://127.0.0.1:8081"
    CATALOG = "demo"
    SCHEMA = "managed_demo"
    TABLE = "events"
    # Managed-storage root for the catalog — must live under the bucket UC has configured
    # (s3.bucketPath.0=s3://olai-demo-1). UC derives each managed table's path beneath this.
    STORAGE_ROOT = "s3://olai-demo-1/managed"
    AWS_REGION = "eu-central-1"
    return AWS_REGION, CATALOG, SCHEMA, STORAGE_ROOT, TABLE, UC_ENDPOINT, UC_URI


@app.cell
def _(CATALOG, SCHEMA, STORAGE_ROOT, UC_URI):
    # Ensure the catalog (WITH a storage_root so managed tables have somewhere to live) + schema
    # exist, via the UC REST API. Idempotent: ALREADY_EXISTS is fine on re-run.
    import requests

    base = f"{UC_URI}/api/2.1/unity-catalog"

    def _create(path, payload):
        r = requests.post(f"{base}/{path}", json=payload)
        if r.status_code not in (200, 201) and "ALREADY_EXISTS" not in r.text:
            r.raise_for_status()
        return r

    _create("catalogs", {"name": CATALOG, "comment": "duckdb demo", "storage_root": STORAGE_ROOT})
    _create("schemas", {"name": SCHEMA, "catalog_name": CATALOG})

    cat = requests.get(f"{base}/catalogs/{CATALOG}").json()
    print("catalog:", cat["name"], "| storage_root:", cat.get("storage_root"))
    return


@app.cell
def _(AWS_REGION, CATALOG, SCHEMA, TABLE, UC_URI):
    # CREATE the managed table with Spark (DuckDB can't do UC DDL). This is the proven path from
    # uc_managed.py: NO LOCATION clause => managed; UC assigns the path and vends creds for the
    # write. The catalogManaged feature flag is required; Delta negotiates the rest of the
    # UC managed contract. Idempotent via IF NOT EXISTS.
    from delta import configure_spark_with_delta_pip
    import pyspark

    builder = (
        pyspark.sql.SparkSession.builder.appName("uc-duckdb-setup")
        .config("spark.sql.extensions", "io.delta.sql.DeltaSparkSessionExtension")
        .config("spark.sql.catalog.spark_catalog", "io.unitycatalog.spark.UCSingleCatalog")
        .config(f"spark.sql.catalog.{CATALOG}", "io.unitycatalog.spark.UCSingleCatalog")
        .config(f"spark.sql.catalog.{CATALOG}.uri", UC_URI)
        .config(f"spark.sql.catalog.{CATALOG}.token", "")  # UC OSS dev: no auth
        .config("spark.sql.defaultCatalog", CATALOG)
        # S3A region only — UC vends STS temp creds; the connector injects them per table.
        .config("spark.hadoop.fs.s3.impl", "org.apache.hadoop.fs.s3a.S3AFileSystem")
        .config("spark.hadoop.fs.s3a.endpoint.region", AWS_REGION)
    )

    extra_packages = [
        "io.unitycatalog:unitycatalog-spark_2.13:0.4.0",
        "org.apache.hadoop:hadoop-aws:3.4.0",
    ]

    spark = configure_spark_with_delta_pip(builder, extra_packages=extra_packages).getOrCreate()
    spark.sparkContext.setLogLevel("WARN")

    spark.sql(
        f"""
        CREATE TABLE IF NOT EXISTS {CATALOG}.{SCHEMA}.{TABLE} (
            id BIGINT, event STRING, ts TIMESTAMP
        ) USING DELTA
        TBLPROPERTIES ('delta.feature.catalogManaged' = 'supported')
        """
    )
    # Seed a couple of rows via Spark so DuckDB has something to read on first run.
    spark.sql(
        f"""
        INSERT INTO {CATALOG}.{SCHEMA}.{TABLE} VALUES
          (1, 'login', TIMESTAMP '2026-06-02 09:00:00'),
          (2, 'click', TIMESTAMP '2026-06-02 09:01:00')
        """
    )
    # Release the table's filesystem/connector handles before DuckDB opens its own writer.
    spark.stop()
    return


@app.cell
def _(AWS_REGION, UC_ENDPOINT):
    # Attach the UC catalog in DuckDB. The unity_catalog extension depends on delta; the UC secret
    # carries the endpoint/region — UC vends the per-table S3 creds, so no separate S3 secret.
    import duckdb

    con = duckdb.connect()
    con.execute("INSTALL unity_catalog; LOAD unity_catalog;")
    con.execute("INSTALL delta; LOAD delta;")

    con.execute(
        f"""
        CREATE OR REPLACE SECRET uc (
            TYPE unity_catalog,
            TOKEN '',
            ENDPOINT '{UC_ENDPOINT}',
            AWS_REGION '{AWS_REGION}'
        )
        """
    )
    return (con, duckdb)


@app.cell
def _(CATALOG, SCHEMA, con):
    # ATTACH the catalog and confirm DuckDB can see it.
    #   - SECRET uc: a *named* secret is only used if ATTACH references it by name (else the
    #     endpoint is empty and requests go out with just the path -> "Could not resolve hostname").
    #   - DEFAULT_SCHEMA: OSS UC can't auto-detect it (that probe hits a Databricks-only endpoint),
    #     so set it explicitly to our schema.
    con.execute(
        f"ATTACH '{CATALOG}' AS uc_demo (TYPE unity_catalog, SECRET uc, DEFAULT_SCHEMA '{SCHEMA}')"
    )
    con.sql("SHOW ALL TABLES")
    return


@app.cell
def _(CATALOG, SCHEMA, TABLE, con):
    # READ the managed table with DuckDB (creds vended by UC).
    con.sql(f"SELECT * FROM uc_demo.{SCHEMA}.{TABLE} ORDER BY id")
    return


@app.cell
def _(CATALOG, SCHEMA, TABLE, con):
    # INSERT more rows through DuckDB. UC append-only: this stages a Delta commit and registers it
    # through UC's Catalog Commits. Wrapping multiple INSERT statements in a transaction collapses them into
    # a single Delta version.
    con.execute("BEGIN")
    con.execute(
        f"""
        INSERT INTO uc_demo.{SCHEMA}.{TABLE} VALUES
          (3, 'scroll',   TIMESTAMP '2026-06-02 09:02:00'),
          (4, 'purchase', TIMESTAMP '2026-06-02 09:03:00')
        """
    )
    con.execute(
        f"""
        INSERT INTO uc_demo.{SCHEMA}.{TABLE} VALUES
          (5, 'logout', TIMESTAMP '2026-06-02 09:05:00')
        """
    )
    con.execute("COMMIT")
    return


@app.cell
def _(SCHEMA, TABLE, con):
    # READ back — DuckDB should now see its own appended rows alongside the Spark-seeded ones.
    con.sql(f"SELECT * FROM uc_demo.{SCHEMA}.{TABLE} ORDER BY id")
    return


@app.cell
def _(SCHEMA, TABLE, con):
    # The append created a new Delta version; time-travel back to the pre-DuckDB snapshot to prove
    # the insert landed as a distinct commit.
    con.sql(f"SELECT count(*) AS rows_at_v0 FROM uc_demo.{SCHEMA}.{TABLE} AT (VERSION => 0)")
    return


if __name__ == "__main__":
    app.run()
