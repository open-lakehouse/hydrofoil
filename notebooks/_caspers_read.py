"""Pluggable SQL read layer for the Casper's stage notebooks.

The stage dashboards express every read as a **SQL query** against the `caspers`
catalog, then run it through a backend chosen at runtime:

  * ``spark``   — a UCSingleCatalog Spark session (the loader's read path), reading
                  UC-managed Delta directly. Exercises the catalog + Delta + S3 stack.
  * ``flight``  — Hydrofoil's Flight SQL endpoint over ADBC, forwarding the chosen
                  principal + UC token (via ``_demo_auth``). Exercises the full
                  governed query path (Cedar + UC + lineage).

Both return a **polars DataFrame**, so the visualization code is backend-agnostic.
This lets one notebook double as a platform test/validation harness: flip
``CASPERS_BACKEND`` between ``spark`` and ``flight`` and confirm the same SQL yields
the same data through both engines.

Carries no PEP 723 block (imported, like ``_demo_auth.py``). The heavy deps (pyspark,
adbc) are imported lazily inside the backend that needs them, so importing this module
is cheap and a notebook only pays for the backend it uses.

Usage in a notebook::

    import _caspers_read as cr
    reader = cr.make_reader()                 # backend from env, defaults to spark
    df = reader.sql("SELECT * FROM caspers.gold.platform_kpis_daily ORDER BY date")
"""

from __future__ import annotations

import os

CATALOG = "caspers"
AWS_REGION = "eu-central-1"
UC_URI = os.environ.get("UC_URI", "http://localhost:8081")
HYDROFOIL_ENDPOINT = os.environ.get("HYDROFOIL_ENDPOINT", "grpc://localhost:50052")


class _SparkReader:
    """Runs SQL via a UCSingleCatalog Spark session; returns polars frames."""

    def __init__(self, namespace: str = "caspers-read"):
        import pyspark

        self._spark = (
            pyspark.sql.SparkSession.builder.appName("caspers-read")
            .config("spark.sql.extensions", "io.delta.sql.DeltaSparkSessionExtension")
            .config("spark.sql.catalog.spark_catalog", "io.unitycatalog.spark.UCSingleCatalog")
            .config(f"spark.sql.catalog.{CATALOG}", "io.unitycatalog.spark.UCSingleCatalog")
            .config(f"spark.sql.catalog.{CATALOG}.uri", UC_URI)
            .config(f"spark.sql.catalog.{CATALOG}.token", "")
            .config("spark.sql.defaultCatalog", CATALOG)
            .config("spark.hadoop.fs.s3.impl", "org.apache.hadoop.fs.s3a.S3AFileSystem")
            .config("spark.hadoop.fs.s3a.endpoint.region", AWS_REGION)
            .config("spark.sql.session.timeZone", "UTC")
            .getOrCreate()
        )
        self._spark.sparkContext.setLogLevel("WARN")

    def sql(self, query: str):
        import polars as pl

        return pl.from_pandas(self._spark.sql(query).toPandas())


class _FlightReader:
    """Runs SQL via Hydrofoil Flight SQL (ADBC); returns polars frames.

    `email` selects the principal whose UC token is forwarded (per `_demo_auth`).
    Optional `lineage` headers (job namespace/name) ride every query so the read
    is attributed in the lineage graph.
    """

    def __init__(self, email: str = "alice@example.com", *, lineage: dict | None = None):
        from adbc_driver_flightsql.dbapi import connect

        import _demo_auth

        self._email = email
        self._conn = connect(
            HYDROFOIL_ENDPOINT,
            db_kwargs=_demo_auth.db_kwargs(email, extra=lineage or {}),
        )

    def sql(self, query: str):
        import polars as pl

        cur = self._conn.cursor()
        try:
            cur.execute(query)
            tbl = cur.fetch_arrow_table()
        finally:
            cur.close()
        return pl.from_arrow(tbl)

    def close(self):
        self._conn.close()


def make_reader(backend: str | None = None, **kwargs):
    """Build a reader. `backend` defaults to env ``CASPERS_BACKEND`` then ``spark``.

    kwargs pass through to the backend (e.g. ``email=`` / ``lineage=`` for flight).
    """
    backend = (backend or os.environ.get("CASPERS_BACKEND", "spark")).lower()
    if backend == "flight":
        return _FlightReader(**kwargs)
    if backend == "spark":
        return _SparkReader(**{k: v for k, v in kwargs.items() if k == "namespace"})
    raise ValueError(f"unknown backend {backend!r} (use 'spark' or 'flight')")
