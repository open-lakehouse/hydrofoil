#!/usr/bin/env python3
"""Tear down the `caspers_test2` test catalog (and ALL its children) via Spark.

This is the destructive counterpart to caspers_load.py: it uses the SAME UC 0.5
Spark connector (io.unitycatalog.spark.UCSingleCatalog) against the deployed Unity
Catalog and drops, bottom-up, every table and schema under `caspers_test2`, then the
catalog itself.

Spark-only by design — if the connector can't DROP SCHEMA / DROP CATALOG against this
UC OSS server, the failing statement and UC error are printed and the exception is
re-raised (non-zero exit). We do NOT silently fall back to the UC REST API.

REQUIRES THE BAKED JARS — UC 0.5 connector, delta-spark, hadoop-aws — i.e. it must run
inside the MARIMO IMAGE. A plain host pyspark has no jars on the classpath and the
session fails to start (ClassNotFoundException on the UC/Delta classes). Run e.g.:

    UC_TOKEN="$(python3 -c 'import sys; sys.path.insert(0,"notebooks"); \
        import _demo_auth; print(_demo_auth.admin_token())')"
    docker run --rm \
        -e UC_URI="https://uc.openlakehousedemos.dev" \
        -e UC_TOKEN="$UC_TOKEN" \
        -e AWS_REGION="us-west-2" \
        -v "$PWD/notebooks:/work" -w /work \
        ghcr.io/open-lakehouse/marimo:marimo-v0.0.9 \
        python /work/drop_caspers_test2.py

Config (matches caspers_load.py; override via env):
    UC_URI      default https://uc.openlakehousedemos.dev
    UC_TOKEN    admin bearer token — env UC_TOKEN/UC_ADMIN_TOKEN, else quickstart .env
    AWS_REGION  default us-west-2
    CATALOG     fixed to caspers_test2 (the test catalog being torn down)
"""

from __future__ import annotations

import os
import sys

import _demo_auth

# The catalog being torn down. Hardcoded to the exact test catalog name so this script
# can only ever target `caspers_test2` — never the real `caspers` data.
CATALOG = "caspers"

# Schemas the server owns that must not be dropped (and can't be).
PROTECTED_SCHEMAS = {"information_schema"}


def main() -> int:
    UC_URI = os.environ.get("UC_URI", "https://uc.openlakehousedemos.dev")
    UC_TOKEN = _demo_auth.admin_token()
    AWS_REGION = os.environ.get("AWS_REGION", "us-west-2")

    # Banner — surface endpoint + auth state up front so an unauthenticated run is never
    # silent. The token VALUE is never printed.
    print(f"UC endpoint:    {UC_URI}")
    print(
        f"auth:           {'Bearer token (set)' if UC_TOKEN else 'NONE — requests are unauthenticated'}"
    )
    print(f"target catalog: {CATALOG}  (and ALL children)")
    print()

    import pyspark

    spark = (
        pyspark.sql.SparkSession.builder.appName("drop-caspers-test2")
        .config("spark.sql.extensions", "io.delta.sql.DeltaSparkSessionExtension")
        .config(
            "spark.sql.catalog.spark_catalog", "io.unitycatalog.spark.UCSingleCatalog"
        )
        .config(f"spark.sql.catalog.{CATALOG}", "io.unitycatalog.spark.UCSingleCatalog")
        .config(f"spark.sql.catalog.{CATALOG}.uri", UC_URI)
        .config(f"spark.sql.catalog.{CATALOG}.token", UC_TOKEN)
        .config("spark.sql.defaultCatalog", CATALOG)
        .config("spark.hadoop.fs.s3.impl", "org.apache.hadoop.fs.s3a.S3AFileSystem")
        .config("spark.hadoop.fs.s3a.endpoint.region", AWS_REGION)
        .config("spark.sql.session.timeZone", "UTC")
        .getOrCreate()
    )
    spark.sparkContext.setLogLevel("WARN")

    def run(sql: str):
        """Run one DDL statement, echoing it. On error, print the statement + re-raise."""
        print(f"  >>> {sql}")
        try:
            return spark.sql(sql)
        except Exception as exc:  # noqa: BLE001 — fail loud, with the failing stmt.
            print(
                f"  !!! FAILED: {sql}\n  !!! {type(exc).__name__}: {exc}",
                file=sys.stderr,
            )
            raise

    # 1. Enumerate schemas under the catalog.
    schemas = [r[0] for r in run(f"SHOW SCHEMAS IN {CATALOG}").collect()]
    schemas = [s for s in schemas if s not in PROTECTED_SCHEMAS]
    print(f"\nschemas to drop: {schemas}\n")

    # 2. drop each table in the schema.
    for schema in schemas:
        tables = [r[1] for r in run(f"SHOW TABLES IN {CATALOG}.{schema}").collect()]
        print(f"  schema {schema}: tables {tables}")
        for table in tables:
            run(f"DROP TABLE {CATALOG}.`{schema}`.`{table}`")
        # run(f"DROP SCHEMA {CATALOG}.`{schema}`")
        print()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
