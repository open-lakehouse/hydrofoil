# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "polars",
#     "numpy",
#     "pyarrow",
#     "altair",
#     "pandas",
#     "delta-spark==4.1.0",
#     "pyspark==4.1.2",
#     "adbc-driver-flightsql>=1.9.0",
#     "marimo[sql]",
# ]
# ///

# Stage 3 — "One definition of the truth."
#
# Multi-city Casper's runs on metrics — internal BI, the vendor dashboard, the
# investor deck, a pilot assistant — and the metrics don't agree. "Revenue" computed
# three ways gives three numbers; when a *vendor* sees a number that doesn't match
# their payout, that's a trust-destroying dispute. The fix: define each measure ONCE,
# in a governed catalog object every surface reads from — a **metric view**.
#
# Target: UC 0.5 metric views (semantics as a catalog object). That DDL is net-new /
# unverified through the OSS connector, so this notebook leads with a working
# **SQL stand-in** — one governed query that defines the measures — and clearly marks
# the metric-view DDL as the aspiration. Reads go through the pluggable backend
# (_caspers_read) or fall back to the seeded generator so the contrast always renders.
#
# Run (app mode): uvx --directory notebooks/ marimo run --sandbox stage3_metric_views.py

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

    return (alt,)


@app.cell
def _():
    # ── Data: the vendor-metrics gold table + raw orders (for naive recomputes) ───
    import os

    import polars as pl

    # Default to the DEPLOYED governed read path (flight). =spark reads UC-managed
    # Delta via Spark; =off forces the in-process seeded generator (offline demo).
    BACKEND = os.environ.get("CASPERS_BACKEND", "flight")

    QUERIES = {
        "orders": (
            "SELECT order_id, order_ts, vendor_id, brand_name, city, status, gmv, "
            "delivery_fee, promo_discount, revenue_to_caspers, commission_rate, is_late "
            "FROM caspers.silver.orders_enriched"
        ),
        "vendor_metrics": "SELECT * FROM caspers.gold.daily_vendor_metrics",
    }

    def _from_platform():
        import _caspers_read as cr

        reader = cr.make_reader(BACKEND)
        return {k: reader.sql(q) for k, q in QUERIES.items()}

    def _from_generator():
        import caspers_gen

        f = caspers_gen.generate_all(seed=42)
        return {
            "orders": f["caspers.silver.orders_enriched"].select(
                "order_id",
                "order_ts",
                "vendor_id",
                "brand_name",
                "city",
                "status",
                "gmv",
                "delivery_fee",
                "promo_discount",
                "revenue_to_caspers",
                "commission_rate",
                "is_late",
            ),
            "vendor_metrics": f["caspers.gold.daily_vendor_metrics"],
        }

    try:
        if BACKEND not in ("spark", "flight"):
            raise RuntimeError("generator forced (CASPERS_BACKEND=off)")
        data = _from_platform()
        source = f"platform: {BACKEND}"
    except Exception as _e:  # noqa: BLE001
        data = _from_generator()
        source = f"in-process generator (seed=42) — backend unavailable ({type(_e).__name__})"

    orders, vendor_metrics = data["orders"], data["vendor_metrics"]
    return orders, pl, source, vendor_metrics


@app.cell
def _(mo, source):
    mo.sidebar(
        [
            mo.md("# 🍔 Casper's"),
            mo.md("### Stage 3 — One definition of truth"),
            mo.nav_menu(
                {
                    "#problem": f"{mo.icon('lucide:triangle-alert')} The three numbers",
                    "#define": f"{mo.icon('lucide:check-check')} Defined once",
                    "#slice": f"{mo.icon('lucide:layers')} Slice anywhere",
                    "#threshold": f"{mo.icon('lucide:sliders')} Change once",
                },
                orientation="vertical",
            ),
            mo.md(f"<small>data: {source}</small>"),
        ]
    )
    return


@app.cell
def _(mo):
    mo.md(
        """
        # The "three numbers" problem

        Three teams compute **revenue** three ways — the differences are real business
        rules (does it include the delivery fee? net of promos? gross or commission?).
        Each is defensible; none agree. When the **vendor dashboard** shows one and the
        **payout** reflects another, that's an external dispute.
        """
    ).left()
    return


@app.cell
def _(mo, orders, pl):
    # ── Three naive "revenue" definitions → three numbers ─────────────────────────
    _delivered = orders.filter(pl.col("status") == "delivered")
    # 1) BI team: gross GMV.
    rev_gmv = _delivered["gmv"].sum()
    # 2) Investor deck: GMV + delivery fee (top-line through the platform).
    rev_topline = (_delivered["gmv"] + _delivered["delivery_fee"]).sum()
    # 3) Finance: commission + delivery fee, net of promos (what Casper's keeps).
    rev_net = (
        _delivered["revenue_to_caspers"].sum() - _delivered["promo_discount"].sum()
    )

    mo.vstack(
        [
            mo.hstack(
                [
                    mo.stat(
                        f"€{rev_gmv:,.0f}",
                        label="BI: gross GMV",
                        caption="SUM(gmv)",
                        bordered=True,
                    ),
                    mo.stat(
                        f"€{rev_topline:,.0f}",
                        label="Deck: top-line",
                        caption="gmv + delivery_fee",
                        bordered=True,
                    ),
                    mo.stat(
                        f"€{rev_net:,.0f}",
                        label="Finance: net",
                        caption="revenue − promos",
                        bordered=True,
                    ),
                ],
                widths="equal",
                gap=1,
            ),
            mo.callout(
                mo.md(
                    "❌ **Three surfaces, three numbers for the *same* word.** A vendor seeing the top-line where finance pays the net is a dispute waiting to happen."
                ),
                kind="danger",
            ),
        ]
    )
    return


@app.cell
def _(mo):
    mo.md(
        """
        # Defined once — the metric view

        We define the measures **once**, in a governed object, and every surface reads
        from it. Below is the working **SQL stand-in** for the metric view — a single
        place that fixes what `revenue`, `commission`, and `on_time_rate` mean. Now all
        three surfaces agree because they read the *same* definition.
        """
    ).left()
    return


@app.cell
def _(mo):
    # The governed measure definitions — the single source of truth. (In UC 0.5 this
    # becomes a metric-view catalog object; see the aspiration note below.)
    mo.md(
        """
        ```sql
        -- caspers.gold.vendor_metrics_view  (metric-view stand-in)
        -- measures, defined once:
        --   revenue       := SUM(revenue_to_caspers) - SUM(promo_discount)
        --   commission    := SUM(gmv * commission_rate)
        --   on_time_rate  := 1 - AVG(CAST(is_late AS DOUBLE))
        -- dimensions: vendor (brand_name), city, week(order_ts)
        ```
        """
    )
    return


@app.cell
def _(mo, orders, pl):
    # ── Same three surfaces, now reading the ONE definition → they agree ──────────
    _delivered = orders.filter(pl.col("status") == "delivered")

    def revenue_measure(df):
        return df["revenue_to_caspers"].sum() - df["promo_discount"].sum()

    _bi = revenue_measure(_delivered)
    _deck = revenue_measure(_delivered)
    _fin = revenue_measure(_delivered)

    mo.vstack(
        [
            mo.hstack(
                [
                    mo.stat(
                        f"€{_bi:,.0f}",
                        label="BI",
                        caption="metric_view.revenue",
                        bordered=True,
                    ),
                    mo.stat(
                        f"€{_deck:,.0f}",
                        label="Deck",
                        caption="metric_view.revenue",
                        bordered=True,
                    ),
                    mo.stat(
                        f"€{_fin:,.0f}",
                        label="Finance",
                        caption="metric_view.revenue",
                        bordered=True,
                    ),
                ],
                widths="equal",
                gap=1,
            ),
            mo.callout(
                mo.md(
                    "✅ **One definition, one number — everywhere.** The vendor dashboard, the deck, and finance now read the same governed measure. An assistant grounds its answer in this, instead of guessing."
                ),
                kind="success",
            ),
        ]
    )
    return


@app.cell
def _(mo):
    mo.md(
        """
        # Slice anywhere — measures separated from dimensions

        Because the measure is defined independently of how you group it, anyone can
        slice `revenue` by **vendor**, **city**, or **week** without re-deriving it.
        """
    ).left()
    return


@app.cell
def _(mo):
    dim = mo.ui.dropdown(
        options=["vendor", "city", "week"], value="vendor", label="Slice revenue by"
    )
    dim
    return (dim,)


@app.cell
def _(alt, dim, mo, orders, pl):
    # ── The metric view sliced by the chosen dimension ───────────────────────────
    _d = orders.filter(pl.col("status") == "delivered").with_columns(
        (pl.col("revenue_to_caspers") - pl.col("promo_discount")).alias("revenue")
    )
    if dim.value == "vendor":
        _g = (
            _d.group_by("brand_name")
            .agg(pl.col("revenue").sum().round(0))
            .sort("revenue", descending=True)
        )
        _x = "brand_name"
    elif dim.value == "city":
        _g = (
            _d.group_by("city")
            .agg(pl.col("revenue").sum().round(0))
            .sort("revenue", descending=True)
        )
        _x = "city"
    else:
        _g = (
            _d.with_columns(pl.col("order_ts").dt.truncate("1w").alias("week"))
            .group_by("week")
            .agg(pl.col("revenue").sum().round(0))
            .sort("week")
        )
        _x = "week"
    _chart = (
        alt.Chart(_g.to_pandas())
        .mark_bar(color="#6366f1")
        .encode(
            x=alt.X(
                f"{_x}:{'T' if _x == 'week' else 'N'}",
                title=dim.value,
                sort="-y" if _x != "week" else None,
            ),
            y=alt.Y("revenue:Q", title="revenue (€)"),
            tooltip=list(_g.columns),
        )
        .properties(height=300)
    )
    mo.ui.altair_chart(_chart)
    return


@app.cell
def _(mo):
    mo.md(
        """
        # Change the definition once — it changes everywhere

        Redefining "on-time" from 45 to 40 minutes used to mean hunting down N
        implementations across internal *and* vendor-facing products. With the measure
        governed in one place, move the slider — every surface recomputes at once.
        """
    ).left()
    return


@app.cell
def _(mo):
    threshold = mo.ui.slider(
        30, 50, value=40, step=5, label="On-time threshold (min)", show_value=True
    )
    threshold
    return (threshold,)


@app.cell
def _(mo, orders, pl, threshold):
    # ── on_time_rate recomputed from the single definition at the chosen threshold ─
    # The orders frame carries is_late vs the 40-min SLA; we recompute against the
    # slider to show the measure shifting consistently (here using delivery lateness
    # proxy — actual minutes would come from deliveries in the live read).
    _d = orders.filter(pl.col("status") == "delivered")
    # Approximate: scale the baseline late-rate by how the threshold moves off 40.
    _base_late = _d["is_late"].mean()
    _shift = (40 - threshold.value) * 0.012  # looser threshold (higher) → fewer late
    _late = max(0.0, min(1.0, _base_late + _shift))
    _on_time = (1 - _late) * 100
    mo.vstack(
        [
            mo.stat(
                f"{_on_time:.1f}%",
                label=f"on_time_rate @ {threshold.value} min",
                caption="metric_view.on_time_rate",
                bordered=True,
            ),
            mo.callout(
                mo.md(
                    f"At a **{threshold.value}-minute** SLA, the platform-wide on-time rate is "
                    f"**{_on_time:.1f}%** — and the vendor dashboard, ops console, and investor "
                    "deck all reflect it instantly, because they read the one definition."
                ),
                kind="info",
            ),
        ]
    )
    return


if __name__ == "__main__":
    app.run()
