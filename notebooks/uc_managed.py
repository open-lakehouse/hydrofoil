# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "delta-spark==4.1.0",
#     "pyspark==4.1.2",
#     "requests",
#     "marimo",
# ]
# ///

# Create a MANAGED Delta table in Unity Catalog (OSS Java) backed by a real AWS S3 bucket.
#
# Managed table = no LOCATION clause. UC assigns the storage path under the catalog's managed
# location (its storage_root) and vends STS-assumed TEMPORARY credentials for the write; the
# UCSingleCatalog connector wires those vended creds into the s3a client. Real AWS S3 accepts
# the session token in those creds (unlike SeaweedFS — see notebooks/uc_crud.py).
#
# Prerequisites:
#   - UC running with the live AWS config (just env-local-up), with the bucket wired via
#     s3.bucketPath.0 / s3.awsRoleArn.0 and `server.managed-table.enabled=true`.
#   - UC reachable at localhost:8081.
#
# Run on the host:
#   uvx --directory notebooks/ marimo edit --sandbox uc_managed.py

import marimo

__generated_with = "0.23.8"
app = marimo.App(width="medium")


@app.cell
def _():
    UC_URI = "http://unity-catalog:8081"
    CATALOG = "demo"
    SCHEMA = "managed_demo"
    TABLE = "events"
    # Managed-storage root for the catalog — must live under the bucket UC has configured
    # (s3.bucketPath.0=s3://olai-demo-1). UC derives each managed table's path beneath this.
    STORAGE_ROOT = "s3://olai-demo-1/managed"
    AWS_REGION = "eu-central-1"
    return AWS_REGION, CATALOG, SCHEMA, STORAGE_ROOT, TABLE, UC_URI


@app.cell
def _(CATALOG, SCHEMA, STORAGE_ROOT, UC_URI):
    # Create catalog (WITH a storage_root so managed tables have somewhere to live) + schema,
    # via the UC REST API. UCSingleCatalog has no CREATE CATALOG/SCHEMA in Spark SQL.
    # Idempotent: ALREADY_EXISTS is fine on re-run.
    import requests

    base = f"{UC_URI}/api/2.1/unity-catalog"

    def _create(path, payload):
        r = requests.post(f"{base}/{path}", json=payload)
        if r.status_code not in (200, 201) and "ALREADY_EXISTS" not in r.text:
            r.raise_for_status()
        return r

    _create("catalogs", {"name": CATALOG, "comment": "managed demo", "storage_root": STORAGE_ROOT})
    _create("schemas", {"name": SCHEMA, "catalog_name": CATALOG})

    cat = requests.get(f"{base}/catalogs/{CATALOG}").json()
    print("catalog:", cat["name"], "| storage_root:", cat.get("storage_root"))
    print(
        "schemas:",
        [s["name"] for s in requests.get(f"{base}/schemas", params={"catalog_name": CATALOG}).json()["schemas"]],
    )
    return


@app.cell
def _(AWS_REGION, CATALOG, UC_URI):
    import pyspark

    # Spark jars (UC 0.5 connector from branch-0.5 + delta-spark + hadoop-aws) are baked
    # onto the classpath in the marimo image — no runtime Ivy resolution / no
    # configure_spark_with_delta_pip.
    spark = (
        pyspark.sql.SparkSession.builder.appName("uc-managed")
        .config("spark.sql.extensions", "io.delta.sql.DeltaSparkSessionExtension")
        .config("spark.sql.catalog.spark_catalog", "io.unitycatalog.spark.UCSingleCatalog")
        .config(f"spark.sql.catalog.{CATALOG}", "io.unitycatalog.spark.UCSingleCatalog")
        .config(f"spark.sql.catalog.{CATALOG}.uri", UC_URI)
        .config(f"spark.sql.catalog.{CATALOG}.token", "")  # UC OSS dev: no auth
        .config("spark.sql.defaultCatalog", CATALOG)
        # S3A region for the real AWS bucket. We do NOT set access/secret keys or a credentials
        # provider here: UC vends STS temporary credentials and the UCSingleCatalog connector
        # injects them into the s3a client per table.
        .config("spark.hadoop.fs.s3.impl", "org.apache.hadoop.fs.s3a.S3AFileSystem")
        .config("spark.hadoop.fs.s3a.endpoint.region", AWS_REGION)
        .getOrCreate()
    )
    spark.sparkContext.setLogLevel("WARN")
    return (spark,)


@app.cell
def _(CATALOG, SCHEMA, TABLE, spark):
    # CREATE a MANAGED table — note: NO LOCATION clause. That is what makes it managed.
    # UC requires the catalog-managed Delta feature flag to be declared; Delta then negotiates
    # the rest of the UC managed contract (v2 checkpoints, in-commit timestamps, ...) itself.
    spark.sql(
        f"""
        CREATE TABLE IF NOT EXISTS {CATALOG}.{SCHEMA}.{TABLE} (
            id BIGINT, event STRING, ts TIMESTAMP
        ) USING DELTA
        TBLPROPERTIES ('delta.feature.catalogManaged' = 'supported')
        """
    )
    spark.sql(
        f"""
        INSERT INTO {CATALOG}.{SCHEMA}.{TABLE} VALUES
          (1, 'login',  TIMESTAMP '2026-06-02 09:00:00'),
          (2, 'click',  TIMESTAMP '2026-06-02 09:01:00'),
          (3, 'logout', TIMESTAMP '2026-06-02 09:05:00')
        """
    )
    return


@app.cell
def _(CATALOG, SCHEMA, TABLE, spark):
    # READ back
    spark.table(f"{CATALOG}.{SCHEMA}.{TABLE}").show(truncate=False)
    return


@app.cell
def _(CATALOG, SCHEMA, TABLE, spark):
    # Confirm it is MANAGED and see the UC-assigned location.
    spark.sql(f"DESCRIBE EXTENDED {CATALOG}.{SCHEMA}.{TABLE}").show(truncate=False)
    return


@app.cell
def _(CATALOG, SCHEMA, spark):
    # LIST
    spark.sql(f"SHOW TABLES IN {CATALOG}.{SCHEMA}").show()
    return


@app.cell
def _(CATALOG, SCHEMA, TABLE, spark):
    # DROP — for a MANAGED table this also removes the underlying data in S3.
    spark.sql(f"DROP TABLE IF EXISTS {CATALOG}.{SCHEMA}.{TABLE}")
    spark.sql(f"SHOW TABLES IN {CATALOG}.{SCHEMA}").show()
    return


if __name__ == "__main__":
    app.run()
