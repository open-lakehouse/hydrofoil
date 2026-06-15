# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "polars",
#     "numpy",
#     "pyarrow",
#     "altair",
#     "plotly",
#     "pandas",
#     "delta-spark==4.1.0",
#     "pyspark==4.1.2",
#     "adbc-driver-flightsql>=1.9.0",
#     "marimo[sql]",
# ]
# ///

# Stage 1 — "We just need to see our marketplace."
#
# Casper's, day one: a young marketplace running on gut feel and a shared bucket of
# app/payment/delivery logs. The founders need to *see* the business — contribution
# margin per order, GMV & growth, the Friday-8pm driver crunch, and which vendors
# actually drive the platform. No governance yet; this is pure access + flexibility.
#
# Every read is expressed as SQL against the `caspers` catalog and run through a
# pluggable backend (Spark over UC-managed Delta, or Hydrofoil Flight SQL) — see
# _caspers_read.py. Set CASPERS_BACKEND=spark|flight. If neither stack is reachable,
# the notebook falls back to regenerating the identical seeded data in-process
# (caspers_gen) so the dashboard always renders for a booth demo.
#
# Run (editor):   uvx --directory notebooks/ marimo edit --sandbox stage1_marketplace.py
# Run (app mode): uvx --directory notebooks/ marimo run  --sandbox stage1_marketplace.py

import marimo

__generated_with = "0.23.8"
app = marimo.App(width="full")


@app.cell
def _():
    import marimo as mo

    return (mo,)


@app.cell
def _():
    # Charting libs imported once (marimo requires each top-level name defined in a
    # single cell); referenced across the dashboard cells.
    import altair as alt
    import plotly.express as px

    return alt, px


@app.cell
def _():
    # ── Data access ────────────────────────────────────────────────────────────
    # Pull the gold/silver tables this dashboard needs. We try the configured
    # platform backend first (Spark or Flight SQL over the caspers catalog); if it's
    # unreachable we regenerate the *identical* seeded data so the demo never blanks.
    import os

    import polars as pl

    BACKEND = os.environ.get("CASPERS_BACKEND", "")  # "spark" | "flight" | "" (auto)

    QUERIES = {
        "kpis": "SELECT * FROM caspers.gold.platform_kpis_daily ORDER BY date",
        "vendors": "SELECT * FROM caspers.gold.daily_vendor_metrics ORDER BY date",
        "zone_time": "SELECT * FROM caspers.gold.zone_time_demand",
        "margin": "SELECT * FROM caspers.gold.contribution_margin_daily",
        "orders": (
            "SELECT order_id, order_ts, zone_id, city, vendor_id, brand_name, status, "
            "gmv, contribution_margin, is_money_losing, dropoff_lat, dropoff_lon, is_late "
            "FROM caspers.silver.orders_enriched"
        ),
    }

    def _from_platform():
        import _caspers_read as cr

        reader = cr.make_reader(BACKEND or "spark")
        return {k: reader.sql(q) for k, q in QUERIES.items()}

    def _from_generator():
        import caspers_gen

        f = caspers_gen.generate_all(seed=42)
        return {
            "kpis": f["caspers.gold.platform_kpis_daily"],
            "vendors": f["caspers.gold.daily_vendor_metrics"],
            "zone_time": f["caspers.gold.zone_time_demand"],
            "margin": f["caspers.gold.contribution_margin_daily"],
            "orders": f["caspers.silver.orders_enriched"].select(
                "order_id", "order_ts", "zone_id", "city", "vendor_id", "brand_name",
                "status", "gmv", "contribution_margin", "is_money_losing",
                "dropoff_lat", "dropoff_lon", "is_late",
            ),
        }

    try:
        if BACKEND in ("spark", "flight"):
            data = _from_platform()
            source = f"platform backend: {BACKEND}"
        else:
            raise RuntimeError("no backend selected")
    except Exception as _e:  # noqa: BLE001
        data = _from_generator()
        source = f"in-process generator (seed=42) — backend unavailable ({type(_e).__name__})"

    kpis, vendors, zone_time, margin, orders = (
        data["kpis"], data["vendors"], data["zone_time"], data["margin"], data["orders"]
    )
    return kpis, margin, orders, pl, source, vendors, zone_time


@app.cell
def _(mo, source):
    # ── Sidebar nav (sleek app-mode chrome) ──────────────────────────────────────
    mo.sidebar(
        [
            mo.md("# 🍔 Casper's"),
            mo.md("### Stage 1 — See our marketplace"),
            mo.nav_menu(
                {
                    "#overview": f"{mo.icon('lucide:home')} Overview",
                    "#crunch": f"{mo.icon('lucide:flame')} The crunch",
                    "#margin": f"{mo.icon('lucide:dollar-sign')} Unit economics",
                    "#vendors": f"{mo.icon('lucide:store')} Vendors",
                },
                orientation="vertical",
            ),
            mo.md(f"<small>data: {source}</small>"),
        ]
    )
    return


@app.cell
def _(kpis, mo, pl):
    # ── Headline KPIs ─────────────────────────────────────────────────────────────
    # Compare the latest 30 days to the prior 30 for direction arrows.
    _k = kpis.sort("date")
    _last = _k.tail(30)
    _prev = _k.tail(60).head(30)

    def _delta(col):
        a, b = _last[col].sum(), _prev[col].sum()
        return "increase" if a >= b else "decrease"

    gmv = _last["gmv"].sum()
    orders_n = _last["orders"].sum()
    take = (_last["revenue"].sum() / max(gmv, 1)) * 100
    margin_total = _last["total_margin"].sum()
    on_time = _last["on_time_rate"].mean() * 100

    mo.hstack(
        [
            mo.stat(f"€{gmv:,.0f}", label="GMV (30d)", caption="gross merchandise value", direction=_delta("gmv"), bordered=True),
            mo.stat(f"{orders_n:,.0f}", label="Orders (30d)", direction=_delta("orders"), bordered=True),
            mo.stat(f"{take:.1f}%", label="Take rate", caption="revenue / GMV", bordered=True),
            mo.stat(f"€{margin_total:,.0f}", label="Contribution margin (30d)", direction=_delta("total_margin"), bordered=True),
            mo.stat(f"{on_time:.0f}%", label="On-time rate", direction="increase", target_direction="increase", bordered=True),
        ],
        widths="equal",
        gap=1,
    )
    return


@app.cell
def _(mo):
    mo.md("## Overview — order volume & growth").left()
    return


@app.cell
def _(alt, kpis, mo, pl):
    # ── GMV / orders growth (brushable) ──────────────────────────────────────────
    _df = kpis.sort("date").select("date", "gmv", "orders").to_pandas()
    _brush = alt.selection_interval(encodings=["x"])
    _base = alt.Chart(_df).encode(x=alt.X("date:T", title=None))
    _gmv = _base.mark_area(opacity=0.35, color="#6366f1").encode(
        y=alt.Y("gmv:Q", title="GMV (€/day)")
    )
    _orders = _base.mark_line(color="#f59e0b").encode(
        y=alt.Y("orders:Q", title="orders/day", axis=alt.Axis(titleColor="#f59e0b"))
    )
    growth = mo.ui.altair_chart(
        alt.layer(_gmv, _orders).resolve_scale(y="independent").add_params(_brush).properties(height=260)
    )
    growth
    return (growth,)


@app.cell
def _(growth, mo, pl):
    # React to the brush: summarize the selected window.
    _sel = growth.value
    if _sel is not None and len(_sel):
        _d = pl.from_pandas(_sel)
        msg = (
            f"**Selected {_d.height} days** — GMV €{_d['gmv'].sum():,.0f}, "
            f"{_d['orders'].sum():,.0f} orders, "
            f"avg €{_d['gmv'].sum()/max(_d['orders'].sum(),1):.2f}/order."
        )
    else:
        msg = "*Drag across the chart to summarize a window.*"
    mo.callout(mo.md(msg), kind="info")
    return


@app.cell
def _(mo):
    mo.md("## The Friday-8pm crunch").left()
    return


@app.cell
def _(mo, pl, px, zone_time):
    # ── Hero: delivery-time heatmap (hour × day-of-week) ──────────────────────────
    # Where driver supply can't meet demand, delivery times spike. The Fri 19–21 cell
    # should glow. Aggregate avg delivery minutes across all zones per (dow, hour).
    _zt = zone_time.with_columns(
        pl.col("date").dt.weekday().alias("dow"),  # Mon=1 … Sun=7 (polars)
    )
    _agg = (
        _zt.group_by(["dow", "hour"]).agg(
            pl.col("avg_delivery_minutes").mean().alias("mins"),
            pl.col("supply_demand_ratio").mean().alias("ratio"),
        )
    ).to_pandas()
    _dow_names = {1: "Mon", 2: "Tue", 3: "Wed", 4: "Thu", 5: "Fri", 6: "Sat", 7: "Sun"}
    _pivot = _agg.pivot(index="dow", columns="hour", values="mins").rename(index=_dow_names)
    _fig = px.imshow(
        _pivot, color_continuous_scale="Inferno", aspect="auto",
        labels=dict(x="hour of day", y="", color="avg min"),
        title="Average delivery time (min) by day × hour",
    )
    _fig.update_layout(template="plotly_dark", height=320, margin=dict(l=40, r=10, t=50, b=30))
    heatmap = mo.ui.plotly(_fig)
    heatmap
    return (heatmap,)


@app.cell
def _(mo, orders, pl, px):
    # ── Late-delivery hotspots map ────────────────────────────────────────────────
    _late = orders.filter((pl.col("status") == "delivered") & pl.col("is_late")).select(
        "dropoff_lat", "dropoff_lon", "city", "zone_id"
    ).to_pandas()
    _fig = px.density_map(
        _late, lat="dropoff_lat", lon="dropoff_lon", radius=8,
        center=dict(lat=52.0, lon=12.0), zoom=4.5, map_style="carto-darkmatter",
        color_continuous_scale="Inferno", title="Late-delivery hotspots",
    )
    _fig.update_layout(template="plotly_dark", height=320, margin=dict(l=0, r=0, t=50, b=0))
    mo.vstack([
        mo.callout(
            mo.md(
                "**The Friday-8pm crunch is real.** When independent-driver supply lags "
                "the dinner-rush demand, delivery times blow past the 40-min SLA — late "
                "deliveries cluster in the map below. This is where Casper's must pull "
                "drivers *before* it bites."
            ),
            kind="warn",
        ),
        mo.ui.plotly(_fig),
    ])
    return


@app.cell
def _(mo):
    mo.md("## Unit economics — are we making money per order?").left()
    return


@app.cell
def _(alt, margin, mo, orders, pl):
    # ── Contribution-margin distribution (the money-losing tail) ──────────────────
    _o = orders.filter(pl.col("status") == "delivered").select("contribution_margin").to_pandas()
    _hist = (
        alt.Chart(_o)
        .mark_bar()
        .encode(
            x=alt.X("contribution_margin:Q", bin=alt.Bin(maxbins=50), title="contribution margin (€/order)"),
            y=alt.Y("count():Q", title="orders"),
            color=alt.condition(
                alt.datum.contribution_margin < 0,
                alt.value("#ef4444"),  # red = money-losing
                alt.value("#22c55e"),  # green = profitable
            ),
        )
        .properties(height=260)
    )
    _losing = orders.filter(pl.col("status") == "delivered")["is_money_losing"].mean() * 100
    # Worst (zone, daypart) cells by share of money-losing orders.
    _worst = (
        margin.group_by(["zone_id", "daypart"]).agg(
            (pl.col("pct_money_losing") * pl.col("orders")).sum().alias("_losers"),
            pl.col("orders").sum().alias("orders"),
        )
        .with_columns((pl.col("_losers") / pl.col("orders")).round(3).alias("pct_losing"))
        .sort("pct_losing", descending=True)
        .head(5)
        .select("zone_id", "daypart", "orders", "pct_losing")
    )
    mo.vstack([
        mo.callout(
            mo.md(
                f"**{_losing:.0f}% of delivered orders lose money.** Heavy acquisition "
                "promos, long outer-zone routes, and crunch surge-payouts push the "
                "margin tail negative — concentrated in the cells below."
            ),
            kind="danger",
        ),
        mo.ui.altair_chart(_hist),
        mo.ui.table(_worst.to_dicts(), label="Worst zone × daypart by money-losing share"),
    ])
    return


@app.cell
def _(mo):
    mo.md("## Which vendors drive the platform?").left()
    return


@app.cell
def _(mo, pl, vendors):
    # ── Vendor leaderboard (selectable) ───────────────────────────────────────────
    _by_vendor = (
        vendors.group_by(["vendor_id", "brand_name"]).agg(
            pl.col("gmv").sum().round(0).alias("gmv"),
            pl.col("orders").sum().alias("orders"),
            pl.col("avg_rating").mean().round(2).alias("avg_rating"),
            pl.col("on_time_rate").mean().round(3).alias("on_time_rate"),
            pl.col("repeat_customer_rate").mean().round(3).alias("repeat_rate"),
        )
        .sort("gmv", descending=True)
    )
    vendor_table = mo.ui.table(
        _by_vendor.to_dicts(),
        label="Vendor leaderboard — select rows to inspect",
        selection="multi",
        format_mapping={"gmv": lambda v: f"€{v:,.0f}"},
    )
    vendor_table
    return (vendor_table,)


@app.cell
def _(alt, mo, pl, vendor_table, vendors):
    # React to vendor selection: GMV trend for the picked brands (or top 2 by default).
    _sel = vendor_table.value
    _ids = [r["vendor_id"] for r in _sel] if _sel else (
        vendors.group_by("vendor_id").agg(pl.col("gmv").sum().alias("g")).sort("g", descending=True).head(2)["vendor_id"].to_list()
    )
    _trend = (
        vendors.filter(pl.col("vendor_id").is_in(_ids))
        .group_by(["date", "brand_name"]).agg(pl.col("gmv").sum().alias("gmv"))
        .sort("date")
        .to_pandas()
    )
    _chart = (
        alt.Chart(_trend)
        .mark_line()
        .encode(
            x=alt.X("date:T", title=None),
            y=alt.Y("gmv:Q", title="GMV (€/day)"),
            color=alt.Color("brand_name:N", title="brand"),
        )
        .properties(height=240)
    )
    mo.vstack([mo.md("### GMV trend for selected vendors"), mo.ui.altair_chart(_chart)])
    return


if __name__ == "__main__":
    app.run()
