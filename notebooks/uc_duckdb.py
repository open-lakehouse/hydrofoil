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

__generated_with = "0.23.8"
app = marimo.App(width="medium")


@app.cell
def _():
    UC_ENDPOINT = "http://unity-catalog:8081"
    CATALOG = "demo"
    SCHEMA = "managed_demo"
    TABLE = "events"
    AWS_REGION = "eu-central-1"
    return AWS_REGION, CATALOG, SCHEMA, TABLE, UC_ENDPOINT


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
    return (con,)


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
    return


@app.cell
def _(con):
    con.sql("SHOW ALL TABLES")
    return


@app.cell
def _(SCHEMA, TABLE, con):
    # READ the managed table with DuckDB (creds vended by UC).
    con.sql(f"SELECT * FROM uc_demo.{SCHEMA}.{TABLE} ORDER BY id")
    return


@app.cell
def _(SCHEMA, TABLE, con):
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


@app.cell
def _():
    import marimo as mo

    return (mo,)


@app.cell
def _(mo):
    _df = mo.sql(
        f"""
        Load unity_catalog;
        FORCE INSTALL delta FROM core_nightly;

        CREATE SECRET (
            TYPE     unity_catalog,
            TOKEN    'demo-ignored-token',
            ENDPOINT 'http://unity-catalog:8081'
        );

        ATTACH 'demo' AS demo
            (TYPE unity_catalog, DEFAULT_SCHEMA 'managed_demo');
        """
    )
    return


@app.cell
def _(mo):
    _df = mo.sql(
        f"""
        SELECT * FROM demo.managed_demo.events;
        """
    )
    return


if __name__ == "__main__":
    app.run()
