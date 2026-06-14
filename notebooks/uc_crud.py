# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "delta-spark==4.1.0",
#     "pyspark==4.1.2",
#     "requests",
#     "marimo",
# ]
# ///

# Basic CRUD against a local Unity Catalog (OSS Java) server.
#
# Catalogs and schemas are created via the UC REST API (the UCSingleCatalog Spark connector
# does not support CREATE CATALOG/SCHEMA via Spark SQL). Table-level CRUD is done with
# PySpark + Delta + the UC Spark connector, writing an EXTERNAL Delta table to SeaweedFS (S3).
#
# Run on the host (matches `just scratch`):
#   just uc-crud
# Expects the environments/ stack up: UC at localhost:8081, SeaweedFS S3 at localhost:9000.
#
# NOTE: the table cells require the UC server to vend S3 credentials WITHOUT a session token
# when pointed at SeaweedFS (SeaweedFS rejects session tokens). See the plan's "Verification
# results" section — this is a one-line fix in the roeap/unitycatalog fork (feat/local-s3).

import marimo

__generated_with = "0.18.4"
app = marimo.App()


@app.cell
def _():
    # Endpoints as seen from the HOST.
    UC_URI = "http://localhost:8081"
    S3_ENDPOINT = "http://localhost:9000"
    ACCESS_KEY = "seaweedfs"
    SECRET_KEY = "seaweedfs"
    CATALOG = "demo"
    SCHEMA = "sales"
    TABLE = "orders"
    BUCKET = "unity"  # created by the seaweedfs-init service
    return (
        ACCESS_KEY,
        BUCKET,
        CATALOG,
        S3_ENDPOINT,
        SCHEMA,
        SECRET_KEY,
        TABLE,
        UC_URI,
    )


@app.cell
def _(ACCESS_KEY, CATALOG, S3_ENDPOINT, SECRET_KEY, UC_URI):
    import pyspark

    # Spark jars (UC 0.5 connector from branch-0.5 + delta-spark + hadoop-aws) are baked
    # onto the classpath in the marimo image — no runtime Ivy resolution / no
    # configure_spark_with_delta_pip.
    spark = (
        pyspark.sql.SparkSession.builder.appName("uc-crud")
        # Delta + UC single-catalog connector
        .config(
            "spark.sql.extensions",
            "io.delta.sql.DeltaSparkSessionExtension",
        )
        .config(
            "spark.sql.catalog.spark_catalog",
            "io.unitycatalog.spark.UCSingleCatalog",
        )
        .config(
            f"spark.sql.catalog.{CATALOG}",
            "io.unitycatalog.spark.UCSingleCatalog",
        )
        .config(f"spark.sql.catalog.{CATALOG}.uri", UC_URI)
        .config(f"spark.sql.catalog.{CATALOG}.token", "")  # UC OSS dev: no auth
        .config("spark.sql.defaultCatalog", CATALOG)
        # --- S3A -> SeaweedFS (bypasses UC credential vending) ---
        .config("spark.hadoop.fs.s3.impl", "org.apache.hadoop.fs.s3a.S3AFileSystem")
        .config("spark.hadoop.fs.s3a.endpoint", S3_ENDPOINT)
        .config("spark.hadoop.fs.s3a.access.key", ACCESS_KEY)
        .config("spark.hadoop.fs.s3a.secret.key", SECRET_KEY)
        # SeaweedFS has no virtual-host bucket addressing -> path style is required.
        .config("spark.hadoop.fs.s3a.path.style.access", "true")
        # Local SeaweedFS S3 is plain HTTP.
        .config("spark.hadoop.fs.s3a.connection.ssl.enabled", "false")
        # Use the static-key provider so it never tries the instance/profile/STS chain.
        .config(
            "spark.hadoop.fs.s3a.aws.credentials.provider",
            "org.apache.hadoop.fs.s3a.SimpleAWSCredentialsProvider",
        )
        .getOrCreate()
    )
    return (spark,)


@app.cell
def _(CATALOG, SCHEMA, UC_URI):
    # CREATE catalog + schema via the UC REST API (UCSingleCatalog has no CREATE CATALOG/SCHEMA
    # in Spark SQL). Idempotent: a 409 ALREADY_EXISTS is fine on re-run.
    import requests

    base = f"{UC_URI}/api/2.1/unity-catalog"

    def _create(path, payload):
        r = requests.post(f"{base}/{path}", json=payload)
        if r.status_code not in (200, 201) and "ALREADY_EXISTS" not in r.text:
            r.raise_for_status()
        return r

    _create("catalogs", {"name": CATALOG, "comment": "crud demo"})
    _create("schemas", {"name": SCHEMA, "catalog_name": CATALOG})

    print("catalogs:", [c["name"] for c in requests.get(f"{base}/catalogs").json()["catalogs"]])
    print(
        "schemas:",
        [s["name"] for s in requests.get(f"{base}/schemas", params={"catalog_name": CATALOG}).json()["schemas"]],
    )
    return


@app.cell
def _(BUCKET, CATALOG, SCHEMA, TABLE, spark):
    # CREATE (write) an EXTERNAL Delta table to SeaweedFS. An explicit LOCATION makes the
    # table external: Spark writes the bytes via s3a, UC only records the path.
    location = f"s3://{BUCKET}/{CATALOG}/{SCHEMA}/{TABLE}"
    spark.sql(
        f"""
        CREATE TABLE IF NOT EXISTS {CATALOG}.{SCHEMA}.{TABLE} (
            id BIGINT, item STRING, qty INT
        ) USING DELTA
        LOCATION '{location}'
        """
    )
    spark.sql(
        f"""
        INSERT INTO {CATALOG}.{SCHEMA}.{TABLE} VALUES
          (1, 'widget', 10), (2, 'gadget', 5), (3, 'gizmo', 7)
        """
    )
    return (location,)


@app.cell
def _(CATALOG, SCHEMA, TABLE, spark):
    # READ back
    df = spark.table(f"{CATALOG}.{SCHEMA}.{TABLE}")
    df.show()
    return (df,)


@app.cell
def _(CATALOG, SCHEMA, spark):
    # LIST
    spark.sql(f"SHOW TABLES IN {CATALOG}.{SCHEMA}").show()
    return


@app.cell
def _(CATALOG, SCHEMA, TABLE, spark):
    # DROP (removes UC metadata; for external tables the s3 data remains).
    spark.sql(f"DROP TABLE IF EXISTS {CATALOG}.{SCHEMA}.{TABLE}")
    spark.sql(f"SHOW TABLES IN {CATALOG}.{SCHEMA}").show()
    return


if __name__ == "__main__":
    app.run()
