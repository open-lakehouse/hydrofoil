# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "adbc-driver-flightsql>=1.9.0",
#     "pyarrow",
#     "marimo",
#     "requests",
# ]
# ///

# Column-level lineage over Flight SQL, end to end.
#
# Hydrofoil's OpenLineage producer resolves *column-level* lineage from the
# optimized DataFusion plan (crates/open-lineage/src/column.rs): a positional
# bottom-up walk that maps every output column of a write to the physical
# (dataset, column) sources it derives from, and attaches the spec's
# `ColumnLineageDatasetFacet` to the OUTPUT dataset. Reads alone carry no
# column lineage (the spec defines the facet on outputs), so this notebook
# runs writes: `INSERT INTO ... SELECT` statements whose expressions exercise
# each transformation class the producer distinguishes:
#
#   DIRECT/IDENTITY        a column passed through untouched (incl. via alias)
#   DIRECT/TRANSFORMATION  any computed expression over columns
#   DIRECT/AGGREGATION     aggregate / window-function arguments
#   INDIRECT/FILTER|JOIN|GROUP_BY|SORT  predicate, join-key, group-key, sort-key
#                          columns that shape the rows without flowing into them
#
# Writes through hydrofoil are authorized by the Cedar gate (`write_table` per
# target table, default-deny — config/policies/lakehouse.cedar permits
# `principal in resource.writers`); the SQLOptions pre-filter no longer blanket-
# rejects DML.
#
# The lineage service persists each event's column-lineage facet
# (`column_lineage_json`) and serves the Marquez column-lineage view back at
# `GET /api/v1/column-lineage?nodeId=dataset:<ns>:<name>` — which this notebook
# renders as a field-level graph.
#
# ── Prerequisites ───────────────────────────────────────────────────────────
#   1. The live stack up, and hydrofoil running ON THE HOST with lineage wired:
#          just env-up       # lineage-service on :8091, Marquez web on :3000
#          just hydro        # host hydrofoil Flight SQL on :50052
#   2. The S3-backed demo table demo.managed_demo.events resolvable via Unity
#      Catalog (the same table duckdb_flight.py queries), plus a writable
#      target table:
#          demo.managed_demo.events_summary (event_type STRING,
#                                            occurrences BIGINT,
#                                            last_id BIGINT)
#      Create it the same way the demo schema was seeded (e.g. the
#      uc_managed.py Spark flow against the same UC). The connecting principal
#      must be in the table's `writers` for the Cedar `write_table` policy.
#
# Run on the host:
#   uvx --directory notebooks/ marimo edit --sandbox column_lineage.py
#
# Afterwards, the Marquez UI (http://localhost:3000) dataset view for
# events_summary shows the column-level graph this notebook renders inline.

import marimo

__generated_with = "0.23.9"
app = marimo.App(width="medium")


@app.cell
def _():
    import uuid

    import marimo as mo

    # Hydrofoil's Flight SQL endpoint: the HOST-run server (`just hydro`,
    # port 50052 per environments/config/live/hydrofoil.toml). The lineage
    # read API is the compose-published lineage-service.
    ENDPOINT = "grpc://localhost:50052"
    LINEAGE_API = "http://localhost:8091/api/v1"

    # Source + target (Unity Catalog managed, real S3).
    SOURCE = "demo.managed_demo.events"
    TARGET = "demo.managed_demo.events_summary"
    TARGET_NAME = TARGET.split(".")[-1]

    NAMESPACE = "column-lineage-demo"
    PARENT_JOB = "column_lineage_walkthrough"
    PARENT_RUN_ID = str(uuid.uuid4())
    return (
        ENDPOINT,
        LINEAGE_API,
        NAMESPACE,
        PARENT_JOB,
        PARENT_RUN_ID,
        SOURCE,
        TARGET,
        TARGET_NAME,
        mo,
    )


@app.cell(hide_code=True)
def _(SOURCE, TARGET, mo):
    mo.md(f"""
    # Column-level lineage over Flight SQL

    This notebook **writes** `{TARGET}` from `{SOURCE}` through hydrofoil and
    reads the resulting **column-level lineage** back from the lineage
    service. Column lineage rides on the *output* dataset (per the OpenLineage
    spec), so each step is an `INSERT INTO … SELECT` whose expressions cover
    the transformation classes the producer distinguishes — identity
    pass-throughs, computed expressions, aggregations, and the indirect
    influences (filter / join / group-by columns).
    """)
    return


@app.cell
def _(ENDPOINT, NAMESPACE, PARENT_JOB, PARENT_RUN_ID):
    from adbc_driver_flightsql import ConnectionOptions, DatabaseOptions
    from adbc_driver_flightsql.dbapi import connect

    HEADER_PREFIX = DatabaseOptions.RPC_CALL_HEADER_PREFIX.value

    # Pipeline-scoped context (see lineage_metadata.py for the full header
    # walkthrough): the principal authorizes the Cedar `write_table` check;
    # the parent facet groups every step under one orchestrator run.
    pipeline_headers = {
        "x-hydrofoil-principal": 'User::"robert.pack"',
        "x-openlineage-job-namespace": NAMESPACE,
        "x-openlineage-parent-run-id": PARENT_RUN_ID,
        "x-openlineage-parent-job-namespace": NAMESPACE,
        "x-openlineage-parent-job-name": PARENT_JOB,
    }

    conn = connect(
        ENDPOINT,
        db_kwargs={
            DatabaseOptions.TLS_SKIP_VERIFY.value: "true",
            **{f"{HEADER_PREFIX}{k}": v for k, v in pipeline_headers.items()},
        },
    )
    return ConnectionOptions, conn


@app.cell
def _(ConnectionOptions, conn):
    STEP_HEADER_KEYS = (
        "x-openlineage-job-name",
        "x-openlineage-job-description",
    )

    def run_step(sql: str, *, job_name: str, description: str = ""):
        """Execute one write step with its OpenLineage job context attached.

        The DoGet stream must be consumed for the write to execute (and the
        terminal lineage event to fire); a DML statement yields a single
        `count` batch, returned here as a pyarrow Table.
        """
        values = dict(zip(STEP_HEADER_KEYS, (job_name, description)))
        prefix = ConnectionOptions.RPC_CALL_HEADER_PREFIX.value
        conn.adbc_connection.set_options(**{f"{prefix}{k}": v for k, v in values.items()})

        cur = conn.cursor()
        try:
            cur.execute(sql)
            return cur.fetch_arrow_table()
        finally:
            cur.close()

    return (run_step,)


@app.cell(hide_code=True)
def _(mo):
    mo.md("""
    ## The writes

    Step 1 aggregates the events table: the group key passes through as
    **identity** (and indirectly as **group-by**), the counts are
    **aggregations**, and the `WHERE` column shows up as an **indirect
    filter** influence on every output field.
    """)
    return


@app.cell
def _(SOURCE, TARGET, run_step):
    summarize = run_step(
        f"""
        INSERT INTO {TARGET}
        SELECT event AS event_type, COUNT(*) AS occurrences, MAX(id) AS last_id
        FROM {SOURCE}
        WHERE id > 0
        GROUP BY event
        """,
        job_name="summarize_events",
        description="Aggregate event volume per type into events_summary.",
    )
    summarize
    return


@app.cell(hide_code=True)
def _(mo):
    mo.md("""
    Step 2 rewrites the summary through a **self-join**: the join keys land as
    indirect **join** influences, the computed `upper(...)` as a
    **transformation** — and because it is the *latest* write of the dataset,
    its facet is the one the column view serves.
    """)
    return


@app.cell
def _(SOURCE, TARGET, run_step):
    rebuild = run_step(
        f"""
        INSERT INTO {TARGET}
        SELECT upper(l.event) AS event_type, COUNT(*) AS occurrences, MAX(r.id) AS last_id
        FROM {SOURCE} l JOIN {SOURCE} r ON l.id = r.id
        WHERE r.id > 1
        GROUP BY upper(l.event)
        """,
        job_name="rebuild_summary",
        description="Self-join rebuild: join keys become INDIRECT/JOIN influences.",
    )
    rebuild
    return


@app.cell(hide_code=True)
def _(mo):
    mo.md("""
    ## Read it back

    The lineage service reconstructs the column view from the latest stored
    facet of the output dataset: `GET /api/v1/column-lineage?nodeId=dataset:…`
    returns `DATASET_FIELD` nodes with edges mirroring each field's
    `inputFields`.
    """)
    return


@app.cell
def _(LINEAGE_API, TARGET_NAME):
    import requests

    # Resolve the dataset's nodeId via search instead of hardcoding the
    # namespace (the producer derives it from hydrofoil's lineage config).
    hits = requests.get(
        f"{LINEAGE_API}/search", params={"q": TARGET_NAME, "limit": 10}, timeout=10
    ).json()
    dataset = next(
        r for r in hits.get("results", [])
        if r.get("type") == "DATASET" and TARGET_NAME in r.get("name", "")
    )
    node_id = f"dataset:{dataset['namespace']}:{dataset['name']}"
    graph = requests.get(
        f"{LINEAGE_API}/column-lineage", params={"nodeId": node_id}, timeout=10
    ).json()["graph"]
    return graph, node_id


@app.cell
def _(graph, mo, node_id):
    # One row per (output field, input field, transformation).
    rows = [
        {
            "output field": node["data"]["field"],
            "input": f"{inp['name']}.{inp['field']}",
            "how": ", ".join(
                f"{t.get('type', '?')}/{t.get('subtype', '?')}"
                for t in inp.get("transformations", [])
            ),
        }
        for node in graph
        if "inputFields" in node["data"]
        for inp in node["data"]["inputFields"]
    ]
    mo.ui.table(rows, label=f"Column lineage of `{node_id}`")
    return


@app.cell
def _(graph, mo):
    # Field-level graph: solid edges for DIRECT provenance, dashed where the
    # source only influences the output indirectly (filter/join/group keys).
    def _nid(s: str) -> str:
        return s.replace(":", "_").replace("/", "_").replace(".", "_").replace("-", "_")

    lines = ["graph LR"]
    for node in graph:
        data = node["data"]
        label = f"{data['dataset']}.{data['field']}"
        lines.append(f'  {_nid(node["id"])}["{label}"]')
        for inp in data.get("inputFields", []):
            origin = _nid(f"datasetField:{inp['namespace']}:{inp['name']}:{inp['field']}")
            kinds = {t.get("type") for t in inp.get("transformations", [])}
            arrow = "-->" if "DIRECT" in kinds else "-.->"
            subtypes = ",".join(
                sorted(t.get("subtype", "") for t in inp.get("transformations", []))
            )
            lines.append(f'  {origin} {arrow}|{subtypes}| {_nid(node["id"])}')
    mo.mermaid("\n".join(lines))
    return


@app.cell(hide_code=True)
def _(mo):
    mo.md("""
    The same graph backs the Marquez UI's dataset **column view**
    (http://localhost:3000 → the summary dataset → columns). Re-running step 1
    flips the view back to its facet — the endpoint always serves the *latest*
    write's column lineage.
    """)
    return


if __name__ == "__main__":
    app.run()
