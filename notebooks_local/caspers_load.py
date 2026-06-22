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
#      feature flag declared) on the catalog's storage_root.
#
# Every Spark write runs with the OpenLineage listener wired to our lineage service
# (namespace `caspers-load`), so the loaded tables become lineage dataset entities
# you can find in the lineage graph / Marquez UI (see notebooks/spark_lineage.py).
#
# Spark jars (UC 0.5 connector from branch-0.5, delta-spark, openlineage-spark,
# hadoop-aws) are baked onto the classpath in the marimo image — no runtime Ivy.
#
# Targets the DEPLOYED open-lakehouse services by default (UC/hydrofoil/lineage on
# *.openlakehousedemos.dev). The UC admin token is resolved automatically from the
# sibling unitycatalog-quickstart .env (or UC_TOKEN/UC_ADMIN_TOKEN env). Override the
# endpoints via env to run against a local stack instead.
#
# Prerequisites (deployed, the default):
#   - The ECS deployment up (it is) and a valid UC admin token reachable (quickstart .env).
#   - storage_root stays UNSET so the deployed UC uses its own managed bucket.
# Prerequisites (local stack, if overriding):
#   - UC at localhost:8081 + lineage-service; set CASPERS_STORAGE_ROOT=s3://olai-demo-1/managed.
#
# Run on the host (from the repo so caspers_gen.py is importable):
#   uvx --directory notebooks/ marimo edit --sandbox caspers_load.py

import marimo

__generated_with = "0.23.9"
app = marimo.App(width="medium")


@app.cell
def _():
    import marimo as mo

    return (mo,)


@app.cell
def _(mo):
    mo.md("""
    # Load Casper's Ghost Kitchen → UC-managed Delta

    Generates the deterministic demo dataset and writes it as **UC-managed Delta
    tables** across four schemas — `caspers.bronze / silver / gold / ml` — on the
    real S3 bucket UC manages. Each write emits **OpenLineage** to our lineage
    service (namespace `caspers-load`) so the tables show up in the lineage graph.
    """)
    return


@app.cell
def _():
    import os

    import _demo_auth

    # Defaults target the DEPLOYED open-lakehouse services (ECS). Override any of these
    # via env to point at a local stack instead (e.g. UC_URI=http://localhost:8081,
    # CASPERS_STORAGE_ROOT=s3://olai-demo-1/managed).
    UC_URI = os.environ.get("UC_URI", "https://uc.openlakehousedemos.dev")
    # UC bearer token — resolved from UC_TOKEN / UC_ADMIN_TOKEN env, else the sibling
    # quickstart .env. The deployed UC has auth enabled, so this is required there;
    # an auth-disabled local UC ignores it.
    UC_TOKEN = _demo_auth.admin_token()
    CATALOG = "caspers"
    SCHEMAS = ["bronze", "silver", "gold", "ml"]
    # Managed-storage root. EMPTY by default so the deployed UC uses its OWN configured
    # managed root (it 403s a catalog pinned to a root it has no external location for —
    # e.g. s3://olai-demo-1). For a local olai-demo-1 stack set
    STORAGE_ROOT = os.environ.get("CASPERS_STORAGE_ROOT", "")
    AWS_REGION = os.environ.get("AWS_REGION", "us-west-2")

    # Lineage service — the deployed URL by default.
    LINEAGE_URL = os.environ.get(
        "LINEAGE_URL", "https://lineage.openlakehousedemos.dev"
    )
    LINEAGE_NAMESPACE = "caspers-load"
    # Register the OpenLineage Spark listener (default on). Set CASPERS_LINEAGE=0 to skip
    # it. NOTE: this Spark path requires the marimo image's baked jars regardless — a
    # jarless host run fails on the Delta/UC classes too, not just OpenLineage.
    WITH_LINEAGE = os.environ.get("CASPERS_LINEAGE", "1") != "0"

    SEED = 42

    HYDROFOIL_ENDPOINT = (
        os.environ.get("HYDROFOIL_ENDPOINT")
        or os.environ.get("HYDROFOIL_GRPC_ENDPOINT")
        or "grpc+tls://hydro-grpc.openlakehousedemos.dev:443"
    )
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
        UC_TOKEN,
        UC_URI,
        WITH_LINEAGE,
    )


@app.cell
def _(SEED, mo):
    # Generate the whole dataset up front (engine-agnostic polars frames).
    import caspers_gen

    frames = caspers_gen.generate_all(seed=SEED)
    mo.ui.table(caspers_gen.table_summary(frames).to_dicts(), label="Generated tables")
    return (frames,)


@app.cell
def _(CATALOG, SCHEMAS, STORAGE_ROOT, UC_TOKEN, UC_URI):
    # Create catalog (with a storage_root if one is set) + schemas via the UC REST API.
    # Idempotent: ALREADY_EXISTS is fine on re-run. UC_TOKEN (if set) authenticates
    # against an auth-enabled server (e.g. the deployed UC).
    import requests

    base = f"{UC_URI}/api/2.1/unity-catalog"
    _headers = {"Authorization": f"Bearer {UC_TOKEN}"} if UC_TOKEN else {}

    # Surface how we're authenticating + what storage_root we'll pin, so an
    # unauthenticated request or an unauthorized root is never silent. The token
    # value itself is never printed.
    print(f"UC endpoint: {UC_URI}")
    print(
        f"auth: {'Bearer token (set)' if UC_TOKEN else 'NONE — requests are unauthenticated'}"
    )
    print(
        f"storage_root: {STORAGE_ROOT or '(unset — server uses its own managed root)'}"
    )

    def _create(path, payload):
        r = requests.post(f"{base}/{path}", json=payload, headers=_headers)
        if r.status_code not in (200, 201) and "ALREADY_EXISTS" not in r.text:
            # Include the response body — a bare raise_for_status() hides the reason
            # (e.g. a 403 PERMISSION_DENIED when storage_root isn't covered by an
            # external location, or a 401 when the token is missing/expired).
            raise RuntimeError(
                f"UC {path} create failed: HTTP {r.status_code} — {r.text}"
            )
        return r

    # Only pin a storage_root when one is configured; otherwise let the server use its
    # own managed root (the deployed UC vends for its own bucket — it 403s a catalog
    # pinned to a root it has no external location for, so set CASPERS_STORAGE_ROOT=""
    # when targeting the deployed UC).
    _cat_payload = {"name": CATALOG, "comment": "Casper's Ghost Kitchen demo"}
    if STORAGE_ROOT:
        _cat_payload["storage_root"] = STORAGE_ROOT
    _create("catalogs", _cat_payload)
    for _schema in SCHEMAS:
        _create("schemas", {"name": _schema, "catalog_name": CATALOG})

    _cat = requests.get(f"{base}/catalogs/{CATALOG}", headers=_headers).json()
    print("catalog:", _cat["name"], "| storage_root:", _cat.get("storage_root"))
    print(
        "schemas:",
        [
            s["name"]
            for s in requests.get(
                f"{base}/schemas", params={"catalog_name": CATALOG}, headers=_headers
            ).json()["schemas"]
        ],
    )
    return


@app.cell
def _(
    AWS_REGION, CATALOG, LINEAGE_NAMESPACE, LINEAGE_URL, UC_TOKEN, UC_URI, WITH_LINEAGE
):
    import pyspark

    # IMPORTANT: this Spark path requires the BAKED JARS — UC 0.5 connector
    # (io.unitycatalog.spark.UCSingleCatalog), delta-spark, hadoop-aws, and (for lineage)
    # openlineage-spark — i.e. it must run inside the MARIMO IMAGE. A plain host
    # `uvx --sandbox` run has bare pyspark with NO jars on the classpath, so the session
    # fails to start with ClassNotFoundException (e.g. OpenLineageSparkListener, or the
    # Delta/UC classes). Run caspers_load.py in the marimo image, not on the host.
    #
    # The OpenLineage listener is gated on WITH_LINEAGE (env CASPERS_LINEAGE, default on)
    # so you can disable lineage explicitly without editing the notebook. It does NOT
    # rescue a jarless host run — the catalog/Delta extensions are required regardless.
    _builder = (
        pyspark.sql.SparkSession.builder.appName("caspers-load")
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
    )
    if WITH_LINEAGE:
        _builder = (
            _builder
            # OpenLineage listener -> our lineage service so loaded tables become entities.
            .config(
                "spark.extraListeners",
                "io.openlineage.spark.agent.OpenLineageSparkListener",
            )
            .config("spark.openlineage.transport.type", "http")
            .config("spark.openlineage.transport.url", LINEAGE_URL)
            .config("spark.openlineage.transport.endpoint", "/api/v1/lineage")
            .config("spark.openlineage.namespace", LINEAGE_NAMESPACE)
            .config("spark.openlineage.columnLineage.datasetLineageEnabled", "true")
        )
    print(
        "OpenLineage listener:",
        "enabled" if WITH_LINEAGE else "disabled (CASPERS_LINEAGE=0)",
    )

    spark = _builder.getOrCreate()
    spark.sparkContext.setLogLevel("WARN")
    return (spark,)


@app.cell
def _(mo):
    mo.md("""
    ## Write each frame as a UC-managed Delta table

    We hand Spark each polars frame via Arrow, then `CREATE TABLE … USING DELTA
    TBLPROPERTIES ('delta.feature.catalogManaged' = 'supported')` (no LOCATION =
    managed) and `INSERT`/overwrite the rows. Re-running is idempotent (seeded data
    + overwrite). Decimal/timestamp types are preserved so the masking and
    classification stories downstream have correct types to work with.
    """)
    return


@app.cell
def _(spark):
    import polars as pl
    import pyspark.sql.functions as F  # noqa: F401 (handy in interactive debugging)
    from pyspark.sql.types import (
        BooleanType,
        DoubleType,
        LongType,
        StringType,
        StructField,
        StructType,
        TimestampType,
    )

    # polars dtype -> Spark type. Integers land as LongType, floats as DoubleType.
    # (We keep money as DoubleType for portability rather than DECIMAL — the connector's
    # Arrow path is happiest with primitive types; the visualizations don't need exact
    # decimal. If exact decimal is wanted later, cast per-column in SQL after load.)
    # NOTE: names must NOT be underscore-prefixed — marimo treats leading-underscore
    # names as cell-private, so they wouldn't be visible to the write cell below.
    def spark_field(name, dtype):
        if dtype in (
            pl.Int8,
            pl.Int16,
            pl.Int32,
            pl.Int64,
            pl.UInt8,
            pl.UInt16,
            pl.UInt32,
            pl.UInt64,
        ):
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

    def to_spark(df_pl):
        # Cast dates to datetimes so Spark sees TIMESTAMP uniformly.
        df_pl = df_pl.with_columns(
            [
                pl.col(c).cast(pl.Datetime("us"))
                for c, d in df_pl.schema.items()
                if d == pl.Date
            ]
        )
        schema = StructType([spark_field(c, d) for c, d in df_pl.schema.items()])
        # Build from ARROW, not pandas: a nullable Int64 column (e.g. driver_id, null on
        # non-delivered orders after the left join) becomes float64 under .to_pandas()
        # (pandas can't hold null in int64), so Spark would get 32.0 for a LongType field
        # and reject it. Arrow preserves nullable int64 exactly, matching the schema.
        return spark.createDataFrame(df_pl.to_arrow(), schema=schema)

    def create_table_ddl(table_name, schema, properties):
        # Explicit-column CREATE TABLE DDL from a Spark StructType. We do NOT use
        # `CREATE TABLE ... AS SELECT` (CTAS): the UC connector's CTAS path does not
        # register the table's columns with the catalog, so the table shows up with no
        # schema. Declaring the columns explicitly (then appending the data) registers
        # them. Mirrors the shared create_table_ddl helper.
        tbl_properties = ""
        if properties:
            props = ", ".join(f"'{k}' = '{v}'" for k, v in properties.items())
            tbl_properties = f"TBLPROPERTIES ({props})"
        col_defs = ", ".join(
            f"{field.name} {field.dataType.simpleString().upper()} "
            f"{'NOT NULL' if not field.nullable else ''}".strip()
            for field in schema.fields
        )
        return (
            f"CREATE TABLE IF NOT EXISTS {table_name}\n"
            f"({col_defs})\n"
            f"USING DELTA\n"
            f"{tbl_properties}"
        ).strip()

    return create_table_ddl, to_spark


@app.cell
def _(create_table_ddl, frames, spark, to_spark):
    # Create each managed table with EXPLICIT columns, then append the rows.
    # NOT `CREATE TABLE ... AS SELECT` (CTAS): the UC connector's CTAS path doesn't
    # register the table's columns with the catalog (the table shows up schema-less),
    # so we declare the columns via create_table_ddl, then write the data with an
    # append. DROP first so re-runs are clean.
    written = []
    for _fq, _pl in frames.items():
        _sdf = to_spark(_pl)
        spark.sql(f"DROP TABLE IF EXISTS {_fq}")
        spark.sql(
            create_table_ddl(
                _fq,
                _sdf.schema,
                {"delta.feature.catalogManaged": "supported"},
            )
        )
        # Append into the just-created (column-registered) managed table.
        _sdf.write.format("delta").mode("append").saveAsTable(_fq)
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


if __name__ == "__main__":
    app.run()
