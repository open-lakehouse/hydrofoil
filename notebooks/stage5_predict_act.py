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

# Stage 5 — "Predict demand, then act on it."
#
# A mature Casper's looks for its next margin lever: extend into the vendors' supply
# chain. Because it sees demand across ALL vendors and cities, it can forecast demand
# (seasonality, weather, day-of-week, local events), run consolidated purchasing, and
# then ACT on the predictions with an autonomous ops agent — pre-order stock, nudge
# vendors, send driver incentives, issue proactive refunds.
#
# This dashboard shows forecast-vs-actual with confidence bands, the demand drivers
# (proving the model has real signal — because the generator baked it in), the
# consolidated wholesale order, and the agent's action feed + hotspot map. It closes
# with the governance question the talk's Act 4 poses (labeled vision): the agent reads
# PII *and* a vendor's forecast, then acts — what stops it leaking one into the other?
#
# Reads via the pluggable backend (_caspers_read) or the seeded generator fallback.
#
# Run (app mode): uvx --directory notebooks/ marimo run --sandbox stage5_predict_act.py

import marimo

__generated_with = "0.23.8"
app = marimo.App(width="full")


@app.cell
def _():
    import marimo as mo

    return (mo,)


@app.cell
def _():
    import altair as alt
    import plotly.express as px

    return alt, px


@app.cell
def _():
    # ── Data: ml forecast/weather/events + agent actions + actuals ────────────────
    import os

    import polars as pl

    # Default to the DEPLOYED governed read path (flight). =spark reads UC-managed
    # Delta via Spark; =off forces the in-process seeded generator (offline demo).
    BACKEND = os.environ.get("CASPERS_BACKEND", "flight")

    QUERIES = {
        "forecast": "SELECT * FROM caspers.ml.demand_forecast",
        "weather": "SELECT * FROM caspers.ml.weather_daily",
        "events": "SELECT * FROM caspers.ml.local_events",
        "actions": "SELECT * FROM caspers.ml.agent_actions",
        "kpis": "SELECT * FROM caspers.gold.platform_kpis_daily ORDER BY date",
        "zone_time": "SELECT zone_id, date, hour, orders, avg_delivery_minutes FROM caspers.gold.zone_time_demand",
    }

    def _from_platform():
        import _caspers_read as cr

        reader = cr.make_reader(BACKEND)
        return {k: reader.sql(q) for k, q in QUERIES.items()}

    def _from_generator():
        import caspers_gen

        f = caspers_gen.generate_all(seed=42)
        return {
            "forecast": f["caspers.ml.demand_forecast"],
            "weather": f["caspers.ml.weather_daily"],
            "events": f["caspers.ml.local_events"],
            "actions": f["caspers.ml.agent_actions"],
            "kpis": f["caspers.gold.platform_kpis_daily"],
            "zone_time": f["caspers.gold.zone_time_demand"].select(
                "zone_id", "date", "hour", "orders", "avg_delivery_minutes"
            ),
        }

    try:
        if BACKEND not in ("spark", "flight"):
            raise RuntimeError("generator forced (CASPERS_BACKEND=off)")
        data = _from_platform()
        source = f"platform: {BACKEND}"
    except Exception as _e:  # noqa: BLE001
        data = _from_generator()
        source = f"in-process generator (seed=42) — backend unavailable ({type(_e).__name__})"

    forecast, weather, actions, kpis, zone_time = (
        data["forecast"], data["weather"], data["actions"], data["kpis"], data["zone_time"]
    )
    return actions, forecast, kpis, pl, source, weather, zone_time


@app.cell
def _(mo, source):
    mo.sidebar(
        [
            mo.md("# 🍔 Casper's"),
            mo.md("### Stage 5 — Predict & act"),
            mo.nav_menu(
                {
                    "#forecast": f"{mo.icon('lucide:trending-up')} Demand forecast",
                    "#drivers": f"{mo.icon('lucide:cloud-sun')} What drives demand",
                    "#purchasing": f"{mo.icon('lucide:shopping-cart')} Consolidated buying",
                    "#agent": f"{mo.icon('lucide:bot')} The agent acts",
                },
                orientation="vertical",
            ),
            mo.md(f"<small>data: {source}</small>"),
        ]
    )
    return


@app.cell
def _(forecast, mo):
    mo.md(
        """
        # Demand forecast — what's popular, where, and when

        Consolidated purchasing only works if Casper's can *forecast* demand. It has
        exactly the data: historical orders plus seasonality, weather, and local
        events. Pick a zone + ingredient to see the 14-day forecast with its 90%
        confidence band.
        """
    ).left()
    return


@app.cell
def _(forecast, mo, pl):
    _zones = sorted(forecast["zone_id"].unique().to_list())
    _ings = sorted(forecast["ingredient_key"].unique().to_list())
    zone_pick = mo.ui.dropdown(_zones, value=_zones[0], label="Zone")
    ing_pick = mo.ui.dropdown(_ings, value=_ings[0], label="Ingredient")
    mo.hstack([zone_pick, ing_pick], justify="start", gap=1)
    return ing_pick, zone_pick


@app.cell
def _(alt, forecast, ing_pick, mo, pl, zone_pick):
    # ── Hero: forecast line + 90% band ────────────────────────────────────────────
    _f = forecast.filter(
        (pl.col("zone_id") == zone_pick.value) & (pl.col("ingredient_key") == ing_pick.value)
    ).sort("forecast_date").to_pandas()

    if len(_f) == 0:
        _out = mo.callout(mo.md("No forecast rows for that combination."), kind="info")
    else:
        _band = (
            alt.Chart(_f)
            .mark_area(opacity=0.25, color="#22c55e")
            .encode(x=alt.X("forecast_date:T", title=None), y=alt.Y("lower_90:Q", title="predicted orders"), y2="upper_90:Q")
        )
        _line = (
            alt.Chart(_f)
            .mark_line(point=True, color="#22c55e")
            .encode(x="forecast_date:T", y="predicted_orders:Q", tooltip=["forecast_date:T", "predicted_orders:Q", "lower_90:Q", "upper_90:Q"])
        )
        _out = mo.ui.altair_chart((_band + _line).properties(height=300))
    mo.vstack([mo.md(f"### {ing_pick.value} demand in {zone_pick.value}"), _out])
    return


@app.cell
def _(mo):
    mo.md(
        """
        # What drives demand — the model has real signal

        The forecast isn't magic: demand moves with **weather** (cold/rain → more
        delivery), **local events**, and **day-of-week**. Casper's models exactly
        these. (This is the textbook lakehouse ML case, trained on the governed estate
        built across Stages 1–4.)
        """
    ).left()
    return


@app.cell
def _(alt, kpis, mo, pl, weather):
    # ── Small multiples: orders vs weather + day-of-week ──────────────────────────
    # Join daily platform orders to citywide-average weather.
    _w = weather.group_by("date").agg(
        pl.col("temp_c").mean().round(1).alias("temp_c"),
        pl.col("precip_mm").mean().round(1).alias("precip_mm"),
    )
    _k = kpis.select("date", "orders").join(_w, on="date", how="left").with_columns(
        pl.col("date").dt.weekday().alias("dow")
    )
    _pd = _k.to_pandas()
    _dow_names = {1: "Mon", 2: "Tue", 3: "Wed", 4: "Thu", 5: "Fri", 6: "Sat", 7: "Sun"}
    _pd["dow_name"] = _pd["dow"].map(_dow_names)

    _temp = alt.Chart(_pd).mark_circle(opacity=0.5, color="#f59e0b").encode(
        x=alt.X("temp_c:Q", title="avg temp (°C)"), y=alt.Y("orders:Q", title="orders/day")
    ).properties(height=220, width=240, title="orders vs temperature")
    _precip = alt.Chart(_pd).mark_circle(opacity=0.5, color="#38bdf8").encode(
        x=alt.X("precip_mm:Q", title="avg precip (mm)"), y=alt.Y("orders:Q", title=None)
    ).properties(height=220, width=240, title="orders vs precipitation")
    _dow = alt.Chart(_pd).mark_boxplot(color="#6366f1").encode(
        x=alt.X("dow_name:N", title="day", sort=list(_dow_names.values())), y=alt.Y("orders:Q", title=None)
    ).properties(height=220, width=240, title="orders by day-of-week")

    mo.ui.altair_chart(alt.hconcat(_temp, _precip, _dow))
    return


@app.cell
def _(mo):
    mo.md(
        """
        # Consolidated purchasing — one wholesale order across all vendors

        Aggregate predicted ingredient demand across every vendor and zone, and Casper's
        can negotiate **better wholesale prices** and run shared logistics — a new
        revenue line and a vendor-retention hook ("join Casper's, pay less for chicken").
        """
    ).left()
    return


@app.cell
def _(forecast, mo, pl):
    # ── Consolidated ingredient order (next 14 days) ──────────────────────────────
    _agg = (
        forecast.group_by("ingredient_key").agg(
            pl.col("predicted_qty").sum().round(0).alias("predicted_qty"),
            pl.col("predicted_orders").sum().round(0).alias("predicted_orders"),
        )
        .sort("predicted_qty", descending=True)
    )
    # Illustrative wholesale saving from consolidation (~8% negotiated).
    _units = _agg["predicted_qty"].sum()
    _saving = _units * 0.35  # € saved per unit at negotiated rate (illustrative)
    mo.vstack([
        mo.hstack(
            [
                mo.stat(f"{_units:,.0f}", label="Units to pre-order (14d)", bordered=True),
                mo.stat(f"€{_saving:,.0f}", label="Est. wholesale saving", caption="~8% negotiated", direction="increase", bordered=True),
                mo.stat(f"{_agg.height}", label="Ingredients consolidated", bordered=True),
            ],
            widths="equal",
            gap=1,
        ),
        mo.ui.table(_agg.to_dicts(), label="Consolidated wholesale order by ingredient"),
    ])
    return


@app.cell
def _(mo):
    mo.md(
        """
        # The agent acts — turning predictions into action

        The bottleneck stopped being data and became *humans acting on it fast enough*.
        Casper's ops agent turns forecasts into action: pre-order stock, nudge vendors,
        target driver incentives at predicted hotspots, issue proactive refunds.
        """
    ).left()
    return


@app.cell
def _(actions, mo, pl, px):
    # ── Agent action feed + hotspot map ───────────────────────────────────────────
    _icons = {
        "preorder_stock": "📦", "nudge_vendor": "📣",
        "driver_incentive": "🛵", "proactive_refund": "💸",
    }
    _feed = actions.sort("acted_ts").head(12).with_columns(
        pl.col("action_type").map_elements(lambda t: f"{_icons.get(t, '•')} {t}", return_dtype=pl.Utf8).alias("action")
    ).select("action", "zone_id", "target_id", "predicted_value", "status", "rationale")

    _map_df = actions.group_by(["zone_id", "ref_lat", "ref_lon"]).agg(
        pl.len().alias("actions"), pl.col("predicted_value").sum().round(0).alias("value")
    ).to_pandas()
    _fig = px.scatter_map(
        _map_df, lat="ref_lat", lon="ref_lon", size="actions", color="value",
        color_continuous_scale="Viridis", size_max=30, zoom=4.5,
        center=dict(lat=52.0, lon=12.0), map_style="carto-darkmatter",
        hover_name="zone_id", title="Where the agent is acting (incentives & pre-orders)",
    )
    _fig.update_layout(template="plotly_dark", height=340, margin=dict(l=0, r=0, t=50, b=0))

    mo.vstack([
        mo.hstack(
            [
                mo.ui.table(_feed.to_dicts(), label="Agent action feed"),
                mo.ui.plotly(_fig),
            ],
            widths=[3, 2],
            gap=1,
        ),
    ])
    return


@app.cell
def _(mo):
    mo.accordion(
        {
            "🔭 Vision: the governance question this raises (Act 4)": mo.md(
                """
                The agent just **read** customer PII (to issue a credit) *and* a vendor's
                forecast (to pre-order) — then it **acts**: messages vendors, moves money,
                contacts customers. An actor that has read personal *and* commercially
                sensitive data and can also *act externally* is, structurally, an
                exfiltration risk — and unlike an employee, it signed no contract and can
                be hijacked by a poisoned input (a crafted review, a malicious vendor
                message).

                None of Casper's existing controls — all **read-time**, all **per-human** —
                even *see* the dangerous question: *the agent read Poke Brand's forecast
                earlier this session; is it allowed to put that into the message it's about
                to send Wing Brand?* Answering it needs tracking what a **session**
                accumulated (taint) and enforcing at the moment of **action** (the tool
                call), not just the read — which lifts governance from the catalog to the
                **platform**. That's the open work. *(Not built yet — this is the talk's
                landing, not a shipped feature.)*
                """
            )
        }
    )
    return


if __name__ == "__main__":
    app.run()
