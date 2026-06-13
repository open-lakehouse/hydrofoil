# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "adbc-driver-flightsql>=1.9.0",
#     "pyarrow",
#     "marimo",
#     "requests",
# ]
# ///

# Rich OpenLineage metadata over Flight SQL, end to end.
#
# Hydrofoil mirrors the OpenLineage *Spark integration*'s configuration surface
# (spark.openlineage.namespace / appName / parent* / job.tags / job.owners) as
# gRPC request metadata, parsed by crates/hydrofoil/src/lineage.rs — see
# docs/adr/0012-client-forwarded-lineage-metadata.md for the header reference.
# This notebook plays the role of an orchestrator running a small "pipeline":
#
#   - connection-level headers carry the *pipeline* context: the principal, the
#     job namespace, and the parent run facet (one orchestrator run id that every
#     step's lineage run parents to);
#   - per-query headers carry each *step*'s context: a meaningful job name,
#     description, tags, owners, and (for one step) an agent task/purpose that
#     lands in the custom `hydrofoil` run facet.
#
# Headers ride the ADBC Flight SQL driver's call-header options
# (`adbc.flight.sql.rpc.call_header.<name>`), settable at connect time
# (DatabaseOptions) and *between queries on one connection*
# (ConnectionOptions via conn.adbc_connection.set_options) — the same channel
# policy_demo.py uses for the principal.
#
# ── Prerequisites ───────────────────────────────────────────────────────────
#   1. The live stack up, and hydrofoil running ON THE HOST with lineage wired:
#          just env-up       # lineage-service on :8091, Marquez web on :3000
#          just hydro        # host hydrofoil Flight SQL on :50052 (the recipe
#                            # overrides lineage.url to the host-published
#                            # http://localhost:8091 — the in-config
#                            # lineage-service hostname only resolves in-compose)
#      The containerized hydrofoil on :50051 runs the released image, which
#      predates these metadata headers — don't point this notebook at it.
#   2. The S3-backed demo table demo.managed_demo.events resolvable via Unity
#      Catalog (the same table duckdb_flight.py queries).
#
# Run on the host:
#   uvx --directory notebooks/ marimo edit --sandbox lineage_metadata.py
#
# Afterwards, the Marquez UI (http://localhost:3000) shows the `demo-pipeline`
# namespace with one job per step — each carrying its description and tags —
# and every run parented to this notebook's orchestrator run.

import marimo

__generated_with = "0.23.8"
app = marimo.App(width="medium")


@app.cell
def _():
    import uuid

    import marimo as mo

    # Hydrofoil's Flight SQL endpoint: the HOST-run server (`just hydro`,
    # port 50052 per environments/config/live/hydrofoil.toml) — NOT the
    # containerized hydrofoil on :50051, whose image predates the metadata
    # headers. The lineage read API is the compose-published lineage-service.
    ENDPOINT = "grpc://localhost:50052"
    LINEAGE_API = "http://localhost:8091/api/v1"

    # The S3-backed demo table (Unity Catalog managed).
    TABLE = "demo.managed_demo.events"

    # The "orchestrator" identity for this notebook session: every step's lineage
    # run will parent to this run id under the demo-pipeline namespace.
    NAMESPACE = "demo-pipeline"
    PARENT_JOB = "nightly_refresh"
    PARENT_RUN_ID = str(uuid.uuid4())
    return ENDPOINT, LINEAGE_API, NAMESPACE, PARENT_JOB, PARENT_RUN_ID, TABLE, mo


@app.cell(hide_code=True)
def _(NAMESPACE, PARENT_RUN_ID, mo):
    mo.md(f"""
    # Rich lineage metadata over Flight SQL

    This notebook acts as an **orchestrator**: it runs a few "pipeline steps"
    against hydrofoil, forwarding OpenLineage context as **gRPC headers** —
    job namespace/name, description, tags, owners, a parent run, and an agent
    task. The server folds them into the emitted events; the lineage service
    and Marquez UI show one job per step under namespace **`{NAMESPACE}`**,
    every run parented to orchestrator run `{PARENT_RUN_ID[:8]}…`.

    | layer | headers |
    |---|---|
    | connection (pipeline) | `x-hydrofoil-principal`, `x-openlineage-job-namespace`, `x-openlineage-parent-*` |
    | per query (step) | `x-openlineage-job-name/-description/-tags/-owners`, `x-hydrofoil-agent-*` |
    """)
    return


@app.cell
def _(ENDPOINT, NAMESPACE, PARENT_JOB, PARENT_RUN_ID):
    from adbc_driver_flightsql import ConnectionOptions, DatabaseOptions
    from adbc_driver_flightsql.dbapi import connect

    HEADER_PREFIX = DatabaseOptions.RPC_CALL_HEADER_PREFIX.value

    # Pipeline-scoped context, fixed for the lifetime of the connection. The
    # parent facet needs all three parent fields (run id, job namespace, job
    # name) — hydrofoil ignores a partial set.
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
    return ConnectionOptions, HEADER_PREFIX, conn


@app.cell
def _(ConnectionOptions, conn):
    # Per-step (per-query) headers: set on the live connection so they apply to
    # the *next* RPCs. Every known step header is written on every call — absent
    # values as "" — so one step's metadata can never leak into the next
    # (hydrofoil treats empty header values as absent).
    STEP_HEADER_KEYS = (
        "x-openlineage-job-name",
        "x-openlineage-job-description",
        "x-openlineage-job-tags",
        "x-openlineage-job-owners",
        "x-hydrofoil-agent-id",
        "x-hydrofoil-agent-task",
        "x-hydrofoil-agent-purpose",
    )

    def run_step(
        sql: str,
        *,
        job_name: str,
        description: str = "",
        tags: str = "",
        owners: str = "",
        agent_id: str = "",
        agent_task: str = "",
        agent_purpose: str = "",
    ):
        """Execute one pipeline step with its OpenLineage context attached.

        `tags` is `key[:value[:source]]` entries and `owners` is `type:name`
        entries, each semicolon-separated (the Spark-parity grammar from ADR
        0012). Returns the result as a pyarrow Table.
        """
        values = dict(
            zip(
                STEP_HEADER_KEYS,
                (job_name, description, tags, owners, agent_id, agent_task, agent_purpose),
            )
        )
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
    ## The pipeline

    Three steps, each a distinct Marquez **job** (its own name, description,
    tags, owners). The summary step runs **twice** — same job, two runs — and
    the last step carries an **agent context** that lands in the custom
    `hydrofoil` run facet alongside the principal.
    """)
    return


@app.cell
def _(TABLE, run_step):
    summary = run_step(
        f"SELECT event, COUNT(*) AS occurrences FROM {TABLE} GROUP BY event ORDER BY occurrences DESC",
        job_name="events_summary",
        description="Daily rollup of event volume per event type.",
        tags="tier:bronze;domain:ops;pii:false",
        owners="team:data-platform;user:robert.pack",
    )
    summary
    return


@app.cell
def _(TABLE, run_step):
    # Re-run the same step: same job name -> the same Marquez job accrues a
    # second run (each with its own runId, both parented to this notebook's
    # orchestrator run).
    rerun = run_step(
        f"SELECT event, COUNT(*) AS occurrences FROM {TABLE} GROUP BY event ORDER BY occurrences DESC",
        job_name="events_summary",
        description="Daily rollup of event volume per event type.",
        tags="tier:bronze;domain:ops;pii:false",
        owners="team:data-platform;user:robert.pack",
    )
    rerun
    return


@app.cell
def _(TABLE, run_step):
    recent = run_step(
        f"SELECT id, event FROM {TABLE} WHERE id > 1 ORDER BY id",
        job_name="recent_events",
        description="Slice of events after the watermark for downstream alerting.",
        tags="tier:silver;domain:ops",
        owners="team:alerting",
    )
    recent
    return


@app.cell
def _(TABLE, run_step):
    # An agent-driven step: the x-hydrofoil-agent-* headers (ADR 0005) are folded
    # into the `hydrofoil` run facet, so the lineage run records on whose behalf
    # and why the query ran.
    agent_step = run_step(
        f"SELECT COUNT(*) AS total FROM {TABLE}",
        job_name="agent_event_count",
        description="Agent-requested sanity count of the events table.",
        tags="tier:bronze;requested-by:agent",
        agent_id="assistant-7",
        agent_task="verify-event-volume",
        agent_purpose="answer 'how many events landed today?'",
    )
    agent_step
    return


@app.cell(hide_code=True)
def _(mo):
    mo.md("""
    ## Read it back

    The lineage service reconstructs the Marquez model from the emitted events.
    Below: the pipeline's **jobs** (with the forwarded description + tags), the
    **runs** of the re-executed step (distinct run ids, one job), and one run's
    **facets** (the `parent` chain and the `hydrofoil` provenance facet).
    """)
    return


@app.cell
def _(LINEAGE_API, NAMESPACE, mo):
    import requests

    jobs = requests.get(f"{LINEAGE_API}/namespaces/{NAMESPACE}/jobs", timeout=10).json()
    mo.ui.table(
        [
            {
                "job": j["name"],
                "description": j.get("description") or "",
                "tags": ", ".join(j.get("tags", [])),
                "runs": len(j.get("latestRuns", [])),
                "state": (j.get("latestRun") or {}).get("state", ""),
            }
            for j in jobs.get("jobs", [])
        ],
        label=f"Jobs in `{NAMESPACE}`",
    )
    return (requests,)


@app.cell
def _(LINEAGE_API, NAMESPACE, mo, requests):
    runs = requests.get(
        f"{LINEAGE_API}/namespaces/{NAMESPACE}/jobs/events_summary/runs", timeout=10
    ).json()
    mo.ui.table(
        [
            {"run_id": r["id"], "state": r["state"], "started": r.get("startedAt") or ""}
            for r in runs.get("runs", [])
        ],
        label="Runs of `events_summary` — re-execution minted distinct run ids",
    )
    return (runs,)


@app.cell
def _(LINEAGE_API, mo, requests, runs):
    latest = runs["runs"][0]["id"] if runs.get("runs") else None
    facets = (
        requests.get(f"{LINEAGE_API}/jobs/runs/{latest}/facets", timeout=10).json()
        if latest
        else {}
    )
    mo.vstack(
        [
            mo.md(f"**Run facets for `{latest}`** — note `parent` (the orchestrator run) "
                  "and `hydrofoil` (principal/agent provenance):"),
            mo.json(facets),
        ]
    )
    return


@app.cell(hide_code=True)
def _(mo):
    mo.md("""
    The raw events (with the full job facets — `documentation`, `tags`,
    `ownership`) are on the events feed: `GET /api/v1/events/lineage`. The
    Marquez web UI renders all of this at <http://localhost:3000>.
    """)
    return


@app.cell
def _(conn):
    conn.close()
    return


if __name__ == "__main__":
    app.run()
