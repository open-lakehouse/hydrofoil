#!/usr/bin/env python3
"""Load Casper's Ghost Kitchen demo data into Unity Catalog **through Hydrofoil**.

A plain script (not a marimo notebook) that:
  1. generates the whole marketplace deterministically (caspers_gen.generate_all), then
  2. for each table, CREATEs a UC-managed Delta table and bulk-ingests the rows —
     both over Hydrofoil's Flight SQL endpoint (ADBC).

Why Hydrofoil instead of the Spark loader (caspers_load.py): the Spark + UC-connector
write path emits OpenLineage *jobs* but no *datasets* (the OpenLineage Spark listener has
no visitor for the UCSingleCatalog V2 write, so dataset facets come out empty). Hydrofoil's
own write path emits proper column-level lineage with input/output datasets — so the loaded
tables show up as datasets in the lineage graph, with columns.

Hydrofoil write contract (crates/hydrofoil, "managed-table bulk ingest + managed CREATE TABLE"):
  * `CREATE TABLE <c>.<s>.<t> (cols) USING DELTA` (no LOCATION) -> managed; UC allocates
    the location. Sent as a Flight SQL update (cursor.execute). CTAS is NOT supported, so
    we CREATE with an explicit column list, then ingest separately.
  * `cursor.adbc_ingest(table, arrow, mode="append")` -> detected as a managed target and
    committed via the catalog's coordinated-commit (UC updateTable AddCommit). Partitioned
    managed tables are not yet supported (we create none).

Per-step OpenLineage metadata (job namespace/name) rides the connection so each table's
create+ingest is attributed under the `caspers-load-hydrofoil` namespace.

Defaults target the DEPLOYED services; override via env (see below). The UC bearer token is
resolved from UC_TOKEN / UC_ADMIN_TOKEN, else the sibling quickstart .env (_demo_auth).

Env:
  HYDROFOIL_ENDPOINT  grpc+tls://hydro-grpc.openlakehousedemos.dev:443 (or HYDROFOIL_GRPC_ENDPOINT)
  UC_TOKEN/UC_ADMIN_TOKEN  bearer token (else read from the quickstart .env)
  CASPERS_PRINCIPAL   demo principal email (default alice@example.com)
  CASPERS_SEED        generator seed (default 42)
  CASPERS_TABLES      comma-separated FQNs to load (default: all)
  CASPERS_RECREATE    "1" to DROP + recreate each table (default: create-if-absent)

Run (needs adbc + the deps caspers_gen uses — easiest in the marimo image, which has them):
  docker run --rm -i \
    -e UC_TOKEN="$UC_ADMIN_TOKEN" \
    -e HYDROFOIL_ENDPOINT="grpc+tls://hydro-grpc.openlakehousedemos.dev:443" \
    -v "$PWD/notebooks":/work:ro -w /work \
    --entrypoint python3 ghcr.io/open-lakehouse/marimo:marimo-v0.0.9 caspers_load_hydrofoil.py
  # or on a host that has polars/numpy/pyarrow/adbc-driver-flightsql:
  #   python3 notebooks/caspers_load_hydrofoil.py
"""

from __future__ import annotations

import os
import sys
import uuid

import polars as pl

import _demo_auth
import caspers_gen

ENDPOINT = (
    os.environ.get("HYDROFOIL_ENDPOINT")
    or os.environ.get("HYDROFOIL_GRPC_ENDPOINT")
    or "grpc+tls://hydro-grpc.openlakehousedemos.dev:443"
)
PRINCIPAL = os.environ.get("CASPERS_PRINCIPAL", "alice@example.com")
# Target catalog. caspers_gen emits FQNs under `caspers.*`; set CASPERS_CATALOG to
# write into a different catalog (e.g. caspers_test) — the prefix is remapped per table.
CATALOG = os.environ.get("CASPERS_CATALOG", "caspers")
SEED = int(os.environ.get("CASPERS_SEED", "42"))
RECREATE = os.environ.get("CASPERS_RECREATE") == "1"
NAMESPACE = os.environ.get("CASPERS_LINEAGE_NAMESPACE", "caspers-load-hydrofoil")
# Belt-and-suspenders lineage: also POST a dataset event (with a schema facet) per
# table directly to the lineage-service REST API. Hydrofoil's managed-ingest now emits
# this itself, but a Hydrofoil that PREDATES that fix emits a job with no output dataset;
# this client-side emit guarantees the table + its columns appear regardless. Set
# CASPERS_EMIT_LINEAGE=0 to skip (e.g. to test the server emission alone).
EMIT_LINEAGE = os.environ.get("CASPERS_EMIT_LINEAGE", "1") != "0"
LINEAGE_API = (
    os.environ.get("LINEAGE_API")
    or (os.environ.get("LINEAGE_URL", "https://lineage.openlakehousedemos.dev").rstrip("/") + "/api/v1")
)
OL_PRODUCER = "https://github.com/open-lakehouse/caspers_load_hydrofoil"
OL_SCHEMA_URL = "https://openlineage.io/spec/2-0-2/OpenLineage.json#/$defs/RunEvent"

# Prefix on per-RPC call-header options (matches _demo_auth).
PFX = _demo_auth.RPC_CALL_HEADER_PREFIX


def sql_type(dtype: pl.DataType) -> str:
    """polars dtype -> Delta/Spark SQL type for the CREATE column list.

    Mirrors caspers_load.py's minimal type set (no DECIMAL / TIMESTAMP_NTZ / nested),
    which keeps the table on the proven UC-managed feature contract our readers support.
    """
    if dtype in (pl.Int8, pl.Int16, pl.Int32, pl.Int64, pl.UInt8, pl.UInt16, pl.UInt32, pl.UInt64):
        return "BIGINT"
    if dtype in (pl.Float32, pl.Float64):
        return "DOUBLE"
    if dtype == pl.Boolean:
        return "BOOLEAN"
    if dtype in (pl.Datetime, pl.Date):
        return "TIMESTAMP"
    return "STRING"


def to_arrow(df: pl.DataFrame):
    """Arrow table for ingest. Dates -> microsecond datetimes so the column is TIMESTAMP
    (matching the CREATE), and Arrow preserves nullable int64 (e.g. driver_id) exactly —
    unlike to_pandas(), which would upcast null-bearing ints to float."""
    df = df.with_columns(
        [pl.col(c).cast(pl.Datetime("us")) for c, d in df.schema.items() if d == pl.Date]
    )
    return df.to_arrow()


def emit_dataset_event(fq: str, df: pl.DataFrame, run_id: str) -> None:
    """POST START+COMPLETE OpenLineage events for `fq` to the lineage-service, with
    the table as an OUTPUT dataset carrying a schema facet (its columns). Best-effort:
    a failure here never fails the load. See the lineage-service smoke test in
    environments/config/deployed/README.md for the accepted event shape."""
    import datetime as _dt
    import json
    import urllib.request

    fields = [{"name": c, "type": str(d)} for c, d in df.schema.items()]
    output = {
        "namespace": NAMESPACE,
        "name": fq,
        "facets": {
            "schema": {
                "_producer": OL_PRODUCER,
                "_schemaURL": "https://openlineage.io/spec/2-0-2/SchemaDatasetFacet.json",
                "fields": fields,
            }
        },
    }
    job_name = f"load_{fq.replace('.', '_')}"
    now = _dt.datetime.now(_dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    for ev_type in ("START", "COMPLETE"):
        body = {
            "eventType": ev_type,
            "eventTime": now,
            "run": {"runId": run_id},
            "job": {"namespace": NAMESPACE, "name": job_name},
            "outputs": [output],
            "producer": OL_PRODUCER,
            "schemaURL": OL_SCHEMA_URL,
        }
        req = urllib.request.Request(
            f"{LINEAGE_API}/lineage",
            data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=10):
            pass


def main() -> int:
    from adbc_driver_flightsql import ConnectionOptions
    from adbc_driver_flightsql.dbapi import connect

    token = _demo_auth.admin_token()
    print(f"Hydrofoil: {ENDPOINT}")
    print(f"principal: {PRINCIPAL} | UC token: {'set' if token else 'NONE (unauthenticated)'}")
    print(f"namespace: {NAMESPACE} | recreate: {RECREATE} | seed: {SEED}")

    frames = caspers_gen.generate_all(seed=SEED)
    # Remap the catalog prefix (caspers.* -> <CATALOG>.*) so we can target caspers_test.
    if CATALOG != "caspers":
        frames = {f"{CATALOG}.{k.split('.', 1)[1]}": v for k, v in frames.items()}
    wanted = os.environ.get("CASPERS_TABLES")
    if wanted:
        keep = {t.strip() for t in wanted.split(",") if t.strip()}
        frames = {k: v for k, v in frames.items() if k in keep}
    print(f"tables to load: {len(frames)}")

    # Connection: principal + UC token (+ admin-token fallback) + the pipeline-scoped
    # lineage namespace so every write is attributed under NAMESPACE.
    kwargs = _demo_auth.db_kwargs(
        PRINCIPAL, extra={"x-openlineage-job-namespace": NAMESPACE}
    )
    token_key = f"{PFX}{_demo_auth.UC_TOKEN_HEADER}"
    if token and token_key not in kwargs:
        kwargs[token_key] = token

    prefix = ConnectionOptions.RPC_CALL_HEADER_PREFIX.value
    written, failed = [], []
    with connect(ENDPOINT, db_kwargs=kwargs) as conn:
        for fq, df in frames.items():
            cols = ", ".join(f"`{c}` {sql_type(d)}" for c, d in df.schema.items())
            # Per-table lineage job name on the live connection (so each table is its own job).
            conn.adbc_connection.set_options(
                **{f"{prefix}x-openlineage-job-name": f"load_{fq.replace('.', '_')}"}
            )
            def run_ddl(c, sql):
                # The DDL's Flight DoGet stream MUST be consumed for the statement to
                # actually execute (Flight SQL executeUpdate semantics) — fetch the
                # result, don't just execute+close, or the CREATE silently no-ops.
                cur = c.cursor()
                try:
                    cur.execute(sql)
                    try:
                        cur.fetch_arrow_table()
                    except Exception:  # noqa: BLE001 — DDL may yield no result set
                        pass
                finally:
                    cur.close()

            try:
                if RECREATE:
                    run_ddl(conn, f"DROP TABLE IF EXISTS {fq}")
                # Managed CREATE: no LOCATION; UC allocates the storage. Hydrofoil's
                # parser accepts `CREATE TABLE <fq> (cols) USING <fmt>` then end-of-
                # statement — NO TBLPROPERTIES (it rejects trailing clauses). The
                # catalogManaged feature is negotiated by the server for managed tables.
                run_ddl(conn, f"CREATE TABLE IF NOT EXISTS {fq} ({cols}) USING DELTA")
                # Bulk-ingest the rows (append) — Hydrofoil detects the managed target and
                # commits via the catalog coordinated-commit, emitting dataset lineage.
                cur = conn.cursor()
                try:
                    n = cur.adbc_ingest(fq, to_arrow(df), mode="append")
                finally:
                    cur.close()
                written.append((fq, n if isinstance(n, int) else df.height))
                print(f"  wrote {fq}: {written[-1][1]} rows")
                # Client-side lineage dataset event (schema facet) — guarantees the
                # table + columns appear even against a Hydrofoil without the
                # server-side managed-ingest emission. Best-effort.
                if EMIT_LINEAGE:
                    try:
                        emit_dataset_event(fq, df, run_id=str(uuid.uuid4()))
                    except Exception as le:  # noqa: BLE001
                        print(f"  (lineage emit skipped for {fq}: {type(le).__name__})", file=sys.stderr)
            except Exception as e:  # noqa: BLE001 — record + continue so one bad table doesn't abort
                failed.append((fq, str(e)))
                print(f"  FAILED {fq}: {type(e).__name__}: {str(e)[:200]}", file=sys.stderr)

    print(f"\n=== DONE: {len(written)} tables written, {len(failed)} failed ===")
    for fq, n in written:
        print(f"  {fq}: {n} rows")
    if failed:
        print("\nfailures:")
        for fq, err in failed:
            print(f"  {fq}: {err[:300]}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
