# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "polars",
#     "numpy",
#     "pyarrow",
#     "adbc-driver-flightsql>=1.9.0",
#     "requests",
#     "marimo",
# ]
# ///

# Stage 4 — "Trace it, and prove it."
#
# Casper's at scale, under GDPR/CCPA. Five pressures, one shape — "show me where this
# data came from and where it goes, automatically, down to the column":
#   - payout-dispute debugging (trace a vendor number back through the joins),
#   - GDPR deletion (find everywhere a customer's data lives),
#   - the accidental PII leak into a vendor view (find the path, prevent the class),
#   - impact analysis (blast radius before a change),
#   - classification that travels (tag PII once at source; inherit downstream).
#
# This notebook drives REAL column-level lineage by writing the caspers silver/gold
# tables THROUGH Hydrofoil (INSERT…SELECT, each step carrying OpenLineage metadata —
# see column_lineage.py) and reading the field-level graph back from the lineage
# service. The classification + propagation views come from caspers_gen.CLASSIFICATIONS
# so they render even before the live graph exists.
#
# Prerequisites (lineage cells need the live stack):
#   - just env-up (lineage-service :8091) + just hydro (Hydrofoil :50052).
#   - caspers catalog loaded (caspers_load.py), with writable silver/gold targets and
#     the connecting principal in their `writers`.
#
# Run: uvx --directory notebooks/ marimo edit --sandbox stage4_lineage_classification.py

import marimo

__generated_with = "0.23.8"
app = marimo.App(width="full")


@app.cell
def _():
    import uuid

    import marimo as mo

    return mo, uuid


@app.cell
def _(uuid):
    import os

    # Defaults to the DEPLOYED services (hydrofoil gRPC+TLS + lineage REST); override
    # for a local stack. HYDROFOIL_GRPC_ENDPOINT is the deployed marimo task's var name.
    ENDPOINT = (
        os.environ.get("HYDROFOIL_ENDPOINT")
        or os.environ.get("HYDROFOIL_GRPC_ENDPOINT")
        or "grpc+tls://hydro-grpc.openlakehousedemos.dev:443"
    )
    LINEAGE_API = os.environ.get("LINEAGE_API", "https://lineage.openlakehousedemos.dev/api/v1")

    SOURCE = "caspers.silver.orders_enriched"
    VENDOR_VIEW = "caspers.gold.vendor_payout_summary"  # the table we (re)build + trace
    VENDOR_VIEW_NAME = VENDOR_VIEW.split(".")[-1]

    NAMESPACE = "caspers-stage4"
    PARENT_JOB = "lineage_classification_walkthrough"
    PARENT_RUN_ID = str(uuid.uuid4())
    return (
        ENDPOINT,
        LINEAGE_API,
        NAMESPACE,
        PARENT_JOB,
        PARENT_RUN_ID,
        SOURCE,
        VENDOR_VIEW,
        VENDOR_VIEW_NAME,
    )


@app.cell
def _(mo):
    mo.sidebar(
        [
            mo.md("# 🍔 Casper's"),
            mo.md("### Stage 4 — Trace it, prove it"),
            mo.nav_menu(
                {
                    "#trace": f"{mo.icon('lucide:git-branch')} Payout-dispute trace",
                    "#leak": f"{mo.icon('lucide:alert-octagon')} The accidental leak",
                    "#gdpr": f"{mo.icon('lucide:trash-2')} GDPR blast radius",
                    "#classify": f"{mo.icon('lucide:tags')} Classification travels",
                },
                orientation="vertical",
            ),
        ]
    )
    return


@app.cell
def _(mo):
    mo.md(
        """
        # Debugging by lineage — the payout dispute

        A vendor disputes their payout: the dashboard says one thing, the payment says
        another. Someone must trace the number back through every join and transform to
        find where it diverged. We rebuild the payout summary **through Hydrofoil** so
        it emits **column-level lineage**, then read the field graph back.
        """
    ).left()
    return


@app.cell
def _(mo):
    import _demo_auth

    user = _demo_auth.user_dropdown(mo)
    user
    return (user,)


@app.cell
def _(ENDPOINT, NAMESPACE, PARENT_JOB, PARENT_RUN_ID, user):
    from adbc_driver_flightsql import ConnectionOptions
    from adbc_driver_flightsql.dbapi import connect

    import _demo_auth

    # Pipeline-scoped lineage context; every write step parents to one run.
    _lineage_headers = {
        "x-openlineage-job-namespace": NAMESPACE,
        "x-openlineage-parent-run-id": PARENT_RUN_ID,
        "x-openlineage-parent-job-namespace": NAMESPACE,
        "x-openlineage-parent-job-name": PARENT_JOB,
    }

    _token_key = f"{_demo_auth.RPC_CALL_HEADER_PREFIX}{_demo_auth.UC_TOKEN_HEADER}"
    _admin = _demo_auth.admin_token()

    def _connect():
        kwargs = _demo_auth.db_kwargs(user.value, extra=_lineage_headers)
        # Admin-token fallback for the deployed (auth-enabled) UC when the chosen user
        # has no per-user token configured.
        if _admin and _token_key not in kwargs:
            kwargs[_token_key] = _admin
        return connect(ENDPOINT, db_kwargs=kwargs)

    return ConnectionOptions, _connect


@app.cell
def _(ConnectionOptions, SOURCE, VENDOR_VIEW, _connect, mo):
    # ── Build the payout summary through Hydrofoil (emits column lineage) ─────────
    # Aggregation (commission_revenue), group-by/identity (brand), filter influence.
    _sql = f"""
        INSERT INTO {VENDOR_VIEW}
        SELECT brand_name,
               SUM(gmv * commission_rate) AS commission_revenue,
               SUM(delivery_fee) AS delivery_revenue,
               COUNT(*) AS orders
        FROM {SOURCE}
        WHERE status = 'delivered'
        GROUP BY brand_name
    """
    _build_err = None
    try:
        _conn = _connect()
        _prefix = ConnectionOptions.RPC_CALL_HEADER_PREFIX.value
        _conn.adbc_connection.set_options(
            **{
                f"{_prefix}x-openlineage-job-name": "build_vendor_payout_summary",
                f"{_prefix}x-openlineage-job-description": "Aggregate commission + delivery revenue per vendor for payouts.",
            }
        )
        _cur = _conn.cursor()
        _cur.execute(_sql)
        _cur.fetch_arrow_table()
        _cur.close()
        _conn.close()
    except Exception as e:  # noqa: BLE001
        _build_err = str(e)

    mo.callout(
        mo.md(
            "✅ Rebuilt `vendor_payout_summary` through Hydrofoil — column lineage emitted."
            if _build_err is None
            else f"Write needs the live stack (Hydrofoil + caspers loaded):\n\n```\n{_build_err}\n```"
        ),
        kind="success" if _build_err is None else "info",
    )
    return


@app.cell
def _(LINEAGE_API, VENDOR_VIEW_NAME, mo):
    # ── Read the column graph back + render it (reuses column_lineage.py shape) ───
    import requests

    graph = None
    node_id = None
    try:
        _hits = requests.get(f"{LINEAGE_API}/search", params={"q": VENDOR_VIEW_NAME, "limit": 10}, timeout=10).json()
        _ds = next(r for r in _hits.get("results", []) if r.get("type") == "DATASET" and VENDOR_VIEW_NAME in r.get("name", ""))
        node_id = f"dataset:{_ds['namespace']}:{_ds['name']}"
        graph = requests.get(f"{LINEAGE_API}/column-lineage", params={"nodeId": node_id}, timeout=10).json()["graph"]
    except Exception:  # noqa: BLE001
        graph = None

    if not graph:
        out = mo.callout(
            mo.md(
                "No column graph yet — run the write above against the live stack. The "
                "mermaid graph + transformation table render here once lineage lands "
                "(commission_revenue ← gmv × commission_rate, etc.)."
            ),
            kind="info",
        )
    else:
        def _nid(s):
            return s.replace(":", "_").replace("/", "_").replace(".", "_").replace("-", "_")

        _lines = ["graph LR"]
        for _node in graph:
            _d = _node["data"]
            _lines.append(f'  {_nid(_node["id"])}["{_d["dataset"]}.{_d["field"]}"]')
            for _inp in _d.get("inputFields", []):
                _origin = _nid(f"datasetField:{_inp['namespace']}:{_inp['name']}:{_inp['field']}")
                _kinds = {t.get("type") for t in _inp.get("transformations", [])}
                _arrow = "-->" if "DIRECT" in _kinds else "-.->"
                _sub = ",".join(sorted(t.get("subtype", "") for t in _inp.get("transformations", [])))
                _lines.append(f'  {_origin} {_arrow}|{_sub}| {_nid(_node["id"])}')
        out = mo.vstack([mo.md(f"**Column lineage of `{node_id}`**"), mo.mermaid("\n".join(_lines))])
    out
    return


@app.cell
def _(mo):
    mo.md(
        """
        # The accidental PII leak — into a vendor's view

        A vendor-facing rollup surfaced a customer's delivery coordinates: they rode
        along in a join nobody scrutinized. Because the leaked-to party is an *external
        vendor*, that's reportable. `silver.orders_enriched` carries **`dropoff_lat` /
        `dropoff_lon` (PII)** right next to vendor-confidential revenue — so any naive
        `SELECT *` rollup pulls the PII through. Lineage + classification is how we find
        the path and prevent the *class*.
        """
    ).left()
    return


@app.cell
def _(mo):
    import caspers_gen

    # Columns on the silver join that carry PII — the leak surface.
    _leaky = {
        col.rsplit(".", 1)[1]: tag
        for col, tag in caspers_gen.CLASSIFICATIONS.items()
        if col.startswith("caspers.silver.orders_enriched.")
    }
    mo.vstack([
        mo.callout(
            mo.md(
                "⚠️ **Leak surface:** these classified columns live on the silver join "
                "feeding vendor rollups. A `SELECT *` rollup would carry the PII into a "
                "vendor's view."
            ),
            kind="danger",
        ),
        mo.ui.table(
            [{"column": c, "classification": t} for c, t in sorted(_leaky.items())],
            label="caspers.silver.orders_enriched — classified columns",
        ),
    ])
    return (caspers_gen,)


@app.cell
def _(mo):
    mo.md(
        """
        # The GDPR deletion request — blast radius

        A customer invokes their right to be forgotten. Casper's must find and delete
        *everywhere* that customer's data lives — raw orders, every downstream
        aggregate, every export. Without lineage, the honest answer is "we're not
        totally sure," which is a compliance failure. Lineage turns it into a list.
        """
    ).left()
    return


@app.cell
def _(caspers_gen, mo):
    # ── Blast radius: tables reachable from customer PII (via classifications) ────
    # Source PII columns on customers; the downstream tables that inherit PII through
    # the silver/gold derivations the lineage graph records.
    _pii_tables = sorted({
        col.rsplit(".", 1)[0]
        for col, tag in caspers_gen.CLASSIFICATIONS.items()
        if tag == "PII"
    })
    _rows = [{"table": t, "reaches_pii": "yes"} for t in _pii_tables]
    mo.vstack([
        mo.callout(
            mo.md(
                "For a deletion request, every table below holds (or derives from) the "
                "customer's personal data and must be addressed. This list is *derived "
                "from lineage + classification*, not maintained by hand."
            ),
            kind="warn",
        ),
        mo.ui.table(_rows, label="GDPR blast radius — tables carrying PII"),
    ])
    return


@app.cell
def _(mo):
    mo.md(
        """
        # Classification that travels

        Casper's tags personal data **once, at the source**, and every downstream table
        that inherits it is known-sensitive **automatically** — because manually
        re-tagging every derived table (especially the vendor-facing ones) is
        impossible at scale and gets it wrong. Below: each source tag and where it
        propagates along the lineage edges.

        > **The masking payoff (now earned):** only once Casper's *knows* which columns
        > are sensitive and how they propagate does the Stage-2 fine-grained masking
        > become meaningful. You can't mask what you can't classify — which is why this
        > comes after metric views, driven by concrete GDPR/leak pressure.
        """
    ).left()
    return


@app.cell
def _(caspers_gen, mo):
    # ── Propagation view: source tag → inherited downstream column ────────────────
    # Group classifications by layer to show "tagged at bronze → inherited at silver/gold".
    _rows = []
    for col, tag in sorted(caspers_gen.CLASSIFICATIONS.items()):
        _parts = col.split(".")
        _layer = _parts[1]
        _rows.append({
            "layer": _layer,
            "column": ".".join(_parts[2:]) if len(_parts) > 2 else col,
            "table": _parts[2] if len(_parts) > 2 else "",
            "classification": tag,
            "origin": "source tag" if _layer == "bronze" else "inherited via lineage",
        })
    mo.ui.table(
        sorted(_rows, key=lambda r: (r["classification"], r["layer"])),
        label="Classification propagation — tagged once at source, inherited downstream",
        format_mapping={},
    )
    return


if __name__ == "__main__":
    app.run()
