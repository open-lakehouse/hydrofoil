# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "polars",
#     "pyarrow",
#     "adbc-driver-flightsql>=1.9.0",
#     "marimo",
# ]
# ///

# Stage 2 — "Teams, vendor-facing data, and a leak scare."
#
# Casper's is funded and selling a vendor-analytics product. Now there are walls to
# enforce: Wing Brand must NOT see Poke Brand's numbers (competitors, sometimes the
# same kitchen); finance needs revenue, not customer PII; and the prod S3 keys must
# come out of human hands. This stage shows the catalog + credential vending +
# per-identity governance answering exactly those needs.
#
# Reads run through HYDROFOIL (Flight SQL), forwarding each principal's identity +
# UC token (via _demo_auth), so Cedar row-filters/column-masks + UC grants apply.
# The SAME SQL returns DIFFERENT results per principal — that's the whole point.
#
# Prerequisites (this stage genuinely needs the live governed stack):
#   - Hydrofoil running with the demo policy + governance feature (see policy_demo.py).
#   - The `caspers` catalog loaded (caspers_load.py).
#   - Per-user UC tokens in notebooks/.env (just mint-demo-tokens) for the vendor /
#     finance principals; emails set via UC_DEMO_USERS.
#
# Run: uvx --directory notebooks/ marimo edit --sandbox stage2_governance.py

import marimo

__generated_with = "0.23.8"
app = marimo.App(width="full")


@app.cell
def _():
    import marimo as mo

    return (mo,)


@app.cell
def _():
    import os

    ENDPOINT = os.environ.get("HYDROFOIL_ENDPOINT", "grpc://localhost:50052")
    # Principals for the walls demo. Vendor principals map to a brand; finance is a
    # revenue-only role. Their UC tokens come from notebooks/.env keyed by email.
    WING = os.environ.get("CASPERS_WING_USER", "wing@example.com")
    POKE = os.environ.get("CASPERS_POKE_USER", "poke@example.com")
    FINANCE = os.environ.get("CASPERS_FINANCE_USER", "finance@example.com")
    FOUNDER = os.environ.get("CASPERS_FOUNDER_USER", "alice@example.com")

    VENDOR_METRICS = "caspers.gold.daily_vendor_metrics"
    ORDERS = "caspers.silver.orders_enriched"
    return ENDPOINT, FINANCE, FOUNDER, ORDERS, POKE, VENDOR_METRICS, WING


@app.cell
def _(ENDPOINT):
    # run_as: execute SQL as a principal over Hydrofoil, forwarding their UC token.
    # Returns (polars_df_or_None, error_str_or_None) — errors (e.g. Cedar deny) are
    # surfaced, not raised, so the dashboard can show the wall in action.
    import polars as pl
    from adbc_driver_flightsql.dbapi import connect

    import _demo_auth

    def run_as(email: str, sql: str):
        try:
            with connect(ENDPOINT, db_kwargs=_demo_auth.db_kwargs(email)) as conn:
                cur = conn.cursor()
                try:
                    cur.execute(sql)
                    return pl.from_arrow(cur.fetch_arrow_table()), None
                finally:
                    cur.close()
        except Exception as e:  # noqa: BLE001 — surface deny/error text
            return None, str(e)

    return pl, run_as


@app.cell
def _(mo):
    mo.sidebar(
        [
            mo.md("# 🍔 Casper's"),
            mo.md("### Stage 2 — Teams & the leak scare"),
            mo.nav_menu(
                {
                    "#walls": f"{mo.icon('lucide:shield')} Vendor walls",
                    "#pii": f"{mo.icon('lucide:eye-off')} Revenue, not PII",
                    "#creds": f"{mo.icon('lucide:key')} Credential vending",
                    "#discover": f"{mo.icon('lucide:search')} Discoverability",
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
        # The resold vendor analytics, with hard walls

        Casper's sells each vendor a dashboard of *their own* performance. The same
        query — `SELECT * FROM caspers.gold.daily_vendor_metrics` — is run as **Wing
        Brand** and as **Poke Brand**. Cedar's per-identity **row filter** must return
        only that vendor's rows. **Wing must never see Poke's numbers** (they compete,
        sometimes in the same kitchen) — and Poke is a *paying* customer of the
        platform.
        """
    ).left()
    return


@app.cell
def _(POKE, VENDOR_METRICS, WING, mo, pl, run_as):
    # ── The wall: same SQL, two vendors, two result sets ──────────────────────────
    _sql = f"SELECT brand_name, COUNT(*) AS days, ROUND(SUM(gmv)) AS gmv, ROUND(AVG(avg_rating),2) AS rating FROM {VENDOR_METRICS} GROUP BY brand_name ORDER BY gmv DESC"

    _wing_df, _wing_err = run_as(WING, _sql)
    _poke_df, _poke_err = run_as(POKE, _sql)

    def _panel(title, df, err):
        if err is not None:
            return mo.vstack([mo.md(f"### {title}"), mo.callout(mo.md(f"```\n{err}\n```"), kind="danger")])
        brands = sorted(set(df["brand_name"].to_list())) if df is not None and df.height else []
        return mo.vstack([
            mo.md(f"### {title}"),
            mo.ui.table(df.to_dicts() if df is not None else [], label=f"brands visible: {brands}"),
        ])

    _leaked = (
        _wing_df is not None and _poke_df is not None
        and len(set(_wing_df["brand_name"].to_list()) & set(_poke_df["brand_name"].to_list())) > 0
    )
    _verdict = (
        mo.callout(mo.md("⚠️ **Wall breach:** both principals see overlapping brands — check the policy + per-user tokens."), kind="danger")
        if _leaked else
        mo.callout(mo.md("✅ **Wall holds:** each vendor sees only their own rows. Wing cannot see Poke."), kind="success")
    )
    mo.vstack([
        mo.hstack([_panel("As Wing Brand", _wing_df, _wing_err), _panel("As Poke Brand", _poke_df, _poke_err)], widths="equal", gap=1),
        _verdict,
    ])
    return


@app.cell
def _(mo):
    mo.md(
        """
        # Finance needs revenue — not a customer's address or card

        Finance computes commission and payouts. They need order totals and vendor
        IDs — they have **no business reason** to see a customer's home coordinates,
        phone, or card. The same `orders` read is run as the **founder** (full access)
        and as **finance**: Cedar's **column mask** should hide the PII columns for
        finance while leaving the revenue columns intact.
        """
    ).left()
    return


@app.cell
def _(FINANCE, FOUNDER, ORDERS, mo, pl, run_as):
    # ── Column masking: founder (unmasked) vs finance (PII masked) ────────────────
    _sql = f"SELECT order_id, brand_name, gmv, revenue_to_caspers, dropoff_lat, dropoff_lon FROM {ORDERS} ORDER BY order_id LIMIT 8"
    _founder_df, _f_err = run_as(FOUNDER, _sql)
    _fin_df, _fin_err = run_as(FINANCE, _sql)

    def _panel(title, df, err, note):
        if err is not None:
            return mo.vstack([mo.md(f"### {title}"), mo.callout(mo.md(f"```\n{err}\n```"), kind="danger")])
        return mo.vstack([mo.md(f"### {title}"), mo.md(f"*{note}*"), mo.ui.table(df.to_dicts() if df is not None else [])])

    mo.hstack(
        [
            _panel("As founder", _founder_df, _f_err, "full access — PII columns visible"),
            _panel("As finance", _fin_df, _fin_err, "PII (dropoff_lat/lon) should be masked; revenue intact"),
        ],
        widths="equal",
        gap=1,
    )
    return


@app.cell
def _(mo):
    mo.md(
        """
        # The leak scare → credential vending

        Someone pasted prod S3 keys into a notebook "just to check something." Nothing
        bad happened — *this time*. The fix isn't a policy memo; it's getting the
        long-lived keys out of human hands entirely. The catalog **vends short-lived,
        scoped STS credentials** per table, per identity — there are no static keys to
        leak (this is exactly how `caspers_load.py` / `uc_managed.py` write to S3: no
        access/secret keys, UC vends temporary creds the connector injects).
        """
    ).left()
    return


@app.cell
def _(mo):
    mo.mermaid(
        """
        flowchart LR
            U["👤 analyst / vendor"] -->|identity + request| C["Unity Catalog"]
            C -->|"short-lived STS creds<br/>(scoped to one table)"| U
            U -->|read with vended creds| S["S3 (caspers managed)"]
            C -. "no static keys<br/>in human hands" .-> X["🔒"]
        """
    )
    return


@app.cell
def _(mo):
    mo.md(
        """
        # Discoverability — one obvious front door

        `orders`, `orders_v2`, `orders_final`… With multiple teams and a vendor-facing
        product, nobody knew which table was canonical — and a dashboard was briefly
        built on a stale copy. The catalog gives one place where the right table is
        obvious. Below: the governed `caspers` estate, layer by layer.
        """
    ).left()
    return


@app.cell
def _(FOUNDER, mo, pl, run_as):
    # ── Discoverability: list the governed estate (best-effort across backends) ───
    _rows = []
    for _schema in ("bronze", "silver", "gold", "ml"):
        _df, _err = run_as(FOUNDER, f"SHOW TABLES IN caspers.{_schema}")
        if _df is not None and _df.height:
            # Column name varies by engine; grab the table-name-ish column.
            _name_col = next((c for c in _df.columns if "table" in c.lower() or c.lower() == "name"), _df.columns[-1])
            for _t in _df[_name_col].to_list():
                _rows.append({"schema": _schema, "table": _t})
    if _rows:
        _out = mo.ui.table(_rows, label="caspers catalog — the governed front door")
    else:
        _out = mo.callout(
            mo.md(
                "Couldn't list tables — needs the live stack with `caspers` loaded. "
                "Run `caspers_load.py` and point `HYDROFOIL_ENDPOINT` at the server."
            ),
            kind="info",
        )
    _out
    return


if __name__ == "__main__":
    app.run()
