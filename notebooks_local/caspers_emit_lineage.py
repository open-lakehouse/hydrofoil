#!/usr/bin/env python3
"""Emit *realistic* transformation lineage for the Casper's Ghost Kitchen demo.

The loaders (caspers_load.py / caspers_load_hydrofoil.py) write every table directly
from pre-computed polars frames, so the only lineage they produce is a flat set of
output datasets — no edges between bronze -> silver -> gold -> ml. This script
reconstructs the lineage you WOULD see if each medallion layer had been computed from
its sources: one OpenLineage RunEvent per derived table, with `inputs` (its source
tables), an output `schema` facet (real columns), and a `columnLineage` facet mapping
output columns to the source columns + transformation that produced them.

It does NOT touch UC or write any data — it only POSTs OpenLineage events to the
lineage-service REST API. Run it AFTER the tables exist (so the dataset nodes already
carry schemas); re-running is idempotent (same job names + fresh run ids per table).

Event format mirrors notebooks_local/caspers_load_hydrofoil.py exactly (same endpoint,
producer, schemaURL, dataset namespace/name convention) so these events line up with
the loader's output datasets rather than creating parallel nodes.

Run (host or container — only needs polars + urllib; the marimo image has polars):
    docker run --rm \
      -e LINEAGE_URL="https://lineage.openlakehousedemos.dev" \
      -e PYTHONPATH="/work:/nb" \
      -v "$PWD/notebooks_local":/work:ro -v "$PWD/notebooks":/nb:ro -w /work \
      --entrypoint python3 ghcr.io/open-lakehouse/marimo:marimo-v0.0.9 \
      caspers_emit_lineage.py
    # or on a host with polars: python3 notebooks_local/caspers_emit_lineage.py

Env:
    LINEAGE_URL / LINEAGE_API   lineage-service base (default deployed)
    CASPERS_CATALOG             target catalog prefix (default caspers)
    CASPERS_LINEAGE_NAMESPACE   dataset/job namespace (default caspers-load)
    CASPERS_SEED                generator seed (default 42; only for real schemas)
"""

from __future__ import annotations

import datetime as dt
import json
import os
import urllib.request
import uuid

import polars as pl

import caspers_gen

LINEAGE_API = os.environ.get("LINEAGE_API") or (
    os.environ.get("LINEAGE_URL", "https://lineage.openlakehousedemos.dev").rstrip("/")
    + "/api/v1"
)
CATALOG = os.environ.get("CASPERS_CATALOG", "caspers")
NAMESPACE = os.environ.get("CASPERS_LINEAGE_NAMESPACE", "caspers-load")
SEED = int(os.environ.get("CASPERS_SEED", "42"))

OL_PRODUCER = "https://github.com/open-lakehouse/caspers_emit_lineage"
OL_SCHEMA_URL = "https://openlineage.io/spec/2-0-2/OpenLineage.json#/$defs/RunEvent"
SCHEMA_FACET_URL = "https://openlineage.io/spec/2-0-2/SchemaDatasetFacet.json"


def fq(table: str) -> str:
    """`bronze.orders` -> `caspers.bronze.orders` (honoring CASPERS_CATALOG)."""
    return f"{CATALOG}.{table}"


# A column-lineage edge: which output column comes from which (table, column) inputs,
# under what transformation. `subtype` follows OpenLineage: IDENTITY (passthrough),
# TRANSFORMATION (derived/computed), AGGREGATION (group-by rollup). `kind` DIRECT means
# the value flows into the column; INDIRECT (used on group-by/filter keys) we mark on
# the dataset-level facet instead.
def col(out, srcs, subtype, desc, masking=False):
    """out: output column; srcs: list of (table, column); -> a column-lineage entry."""
    return (out, srcs, subtype, desc, masking)


# The medallion DAG. For each derived table: its source tables (drive `inputs`) and the
# per-column mappings (drive the `columnLineage` facet). Columns not listed still appear
# via the schema facet — they just carry no explicit upstream edge. Mappings reflect the
# actual generator logic in caspers_gen.py (joins + computed expressions).
DERIVATIONS: dict[str, dict] = {
    # ---- SILVER: clean / conform / enrich -----------------------------------------
    "silver.customers_clean": {
        "inputs": ["bronze.customers"],
        "desc": "Conform + lightly clean the raw customer table (PII reference).",
        "columns": [
            col("customer_id", [("bronze.customers", "customer_id")], "IDENTITY", ""),
            col("full_name", [("bronze.customers", "full_name")], "IDENTITY", "", masking=True),
            col("email", [("bronze.customers", "email")], "IDENTITY", "", masking=True),
            col("phone", [("bronze.customers", "phone")], "IDENTITY", "", masking=True),
            col("zone_id", [("bronze.customers", "zone_id")], "IDENTITY", ""),
            col("cohort", [("bronze.customers", "cohort")], "IDENTITY", ""),
        ],
    },
    "silver.orders_enriched": {
        "inputs": [
            "bronze.orders",
            "bronze.vendors",
            "bronze.payments",
            "bronze.deliveries",
        ],
        "desc": "Join orders to vendor/payment/delivery facts and compute order economics.",
        "columns": [
            col("order_id", [("bronze.orders", "order_id")], "IDENTITY", ""),
            col("customer_id", [("bronze.orders", "customer_id")], "IDENTITY", ""),
            col("vendor_id", [("bronze.orders", "vendor_id")], "IDENTITY", ""),
            col("brand_name", [("bronze.vendors", "brand_name")], "IDENTITY", "join on vendor_id"),
            col("cuisine", [("bronze.vendors", "cuisine")], "IDENTITY", "join on vendor_id"),
            col("commission_rate", [("bronze.vendors", "commission_rate")], "IDENTITY", "join on vendor_id"),
            col("processor_fee", [("bronze.payments", "processor_fee")], "IDENTITY", "join on order_id"),
            col("driver_payout", [("bronze.deliveries", "driver_payout")], "IDENTITY", "join on order_id"),
            col("actual_minutes", [("bronze.deliveries", "actual_minutes")], "IDENTITY", "join on order_id"),
            col("is_late", [("bronze.deliveries", "is_late")], "IDENTITY", "join on order_id"),
            col(
                "revenue_to_caspers",
                [
                    ("bronze.orders", "gmv"),
                    ("bronze.vendors", "commission_rate"),
                    ("bronze.orders", "delivery_fee"),
                ],
                "TRANSFORMATION",
                "(gmv * commission_rate) + delivery_fee",
            ),
            col(
                "contribution_margin",
                [
                    ("bronze.orders", "gmv"),
                    ("bronze.vendors", "commission_rate"),
                    ("bronze.orders", "delivery_fee"),
                    ("bronze.payments", "processor_fee"),
                    ("bronze.deliveries", "driver_payout"),
                    ("bronze.orders", "promo_discount"),
                ],
                "TRANSFORMATION",
                "(gmv*commission_rate)+delivery_fee - processor_fee - driver_payout - promo_discount",
            ),
            col(
                "is_money_losing",
                [
                    ("bronze.orders", "gmv"),
                    ("bronze.orders", "promo_discount"),
                    ("bronze.deliveries", "driver_payout"),
                ],
                "TRANSFORMATION",
                "contribution_margin < 0",
            ),
        ],
    },
    "silver.deliveries_conformed": {
        "inputs": ["bronze.deliveries", "bronze.orders"],
        "desc": "Conform deliveries and derive time-bucket features from the order timestamp.",
        "columns": [
            col("delivery_id", [("bronze.deliveries", "delivery_id")], "IDENTITY", ""),
            col("order_id", [("bronze.deliveries", "order_id")], "IDENTITY", ""),
            col("driver_id", [("bronze.deliveries", "driver_id")], "IDENTITY", ""),
            col("actual_minutes", [("bronze.deliveries", "actual_minutes")], "IDENTITY", ""),
            col("is_late", [("bronze.deliveries", "is_late")], "IDENTITY", ""),
            col("order_ts", [("bronze.orders", "order_ts")], "IDENTITY", "join on order_id"),
            col("city", [("bronze.orders", "city")], "IDENTITY", "join on order_id"),
            col("hour_bucket", [("bronze.orders", "order_ts")], "TRANSFORMATION", "hour(order_ts)"),
            col("dow", [("bronze.orders", "order_ts")], "TRANSFORMATION", "weekday(order_ts)"),
            col("is_friday_peak", [("bronze.orders", "order_ts")], "TRANSFORMATION", "dow==Fri AND hour in 19..21"),
        ],
    },
    # ---- GOLD: aggregated metrics --------------------------------------------------
    "gold.daily_vendor_metrics": {
        "inputs": ["silver.orders_enriched", "bronze.ratings"],
        "desc": "Daily per-vendor rollup of orders, GMV, commission and ratings.",
        "columns": [
            col("date", [("silver.orders_enriched", "order_ts")], "TRANSFORMATION", "date(order_ts) [group key]"),
            col("vendor_id", [("silver.orders_enriched", "vendor_id")], "IDENTITY", "group key"),
            col("brand_name", [("silver.orders_enriched", "brand_name")], "IDENTITY", "group key"),
            col("orders", [("silver.orders_enriched", "order_id")], "AGGREGATION", "count() by (date, vendor_id)"),
            col("gmv", [("silver.orders_enriched", "gmv")], "AGGREGATION", "sum(gmv)"),
            col("commission_revenue", [("silver.orders_enriched", "revenue_to_caspers")], "AGGREGATION", "sum(revenue_to_caspers)"),
            col("avg_rating", [("bronze.ratings", "stars")], "AGGREGATION", "mean(stars) via left-join on order_id"),
            col("on_time_rate", [("silver.orders_enriched", "is_late")], "AGGREGATION", "1 - mean(is_late)"),
        ],
    },
    "gold.zone_time_demand": {
        "inputs": ["silver.orders_enriched"],
        "desc": "Zone x date x hour demand vs. driver supply; flags the Friday-evening crunch.",
        "columns": [
            col("zone_id", [("silver.orders_enriched", "zone_id")], "IDENTITY", "group key"),
            col("date", [("silver.orders_enriched", "order_ts")], "TRANSFORMATION", "date(order_ts) [group key]"),
            col("hour", [("silver.orders_enriched", "order_ts")], "TRANSFORMATION", "hour(order_ts) [group key]"),
            col("orders", [("silver.orders_enriched", "order_id")], "AGGREGATION", "count() by (zone, date, hour)"),
            col("avg_delivery_minutes", [("silver.orders_enriched", "actual_minutes")], "AGGREGATION", "mean(actual_minutes)"),
            col("late_rate", [("silver.orders_enriched", "is_late")], "AGGREGATION", "mean(is_late)"),
            col("supply_demand_ratio", [("silver.orders_enriched", "order_id")], "TRANSFORMATION", "active_drivers / max(orders, 1)"),
            col("is_crunch", [("silver.orders_enriched", "order_id")], "TRANSFORMATION", "supply_demand_ratio < 0.8 AND orders >= 2"),
        ],
    },
    "gold.contribution_margin_daily": {
        "inputs": ["silver.orders_enriched"],
        "desc": "Daily margin by zone and daypart; surfaces the money-losing tail.",
        "columns": [
            col("date", [("silver.orders_enriched", "order_ts")], "TRANSFORMATION", "date(order_ts) [group key]"),
            col("zone_id", [("silver.orders_enriched", "zone_id")], "IDENTITY", "group key"),
            col("daypart", [("silver.orders_enriched", "order_ts")], "TRANSFORMATION", "bucket hour(order_ts) into daypart [group key]"),
            col("orders", [("silver.orders_enriched", "order_id")], "AGGREGATION", "count() by (date, zone, daypart)"),
            col("gmv", [("silver.orders_enriched", "gmv")], "AGGREGATION", "sum(gmv)"),
            col("total_margin", [("silver.orders_enriched", "contribution_margin")], "AGGREGATION", "sum(contribution_margin)"),
            col("avg_margin_per_order", [("silver.orders_enriched", "contribution_margin")], "AGGREGATION", "mean(contribution_margin)"),
            col("pct_money_losing", [("silver.orders_enriched", "is_money_losing")], "AGGREGATION", "mean(is_money_losing)"),
        ],
    },
    "gold.platform_kpis_daily": {
        "inputs": ["silver.orders_enriched", "silver.deliveries_conformed"],
        "desc": "Top-line daily platform KPIs.",
        "columns": [
            col("date", [("silver.orders_enriched", "order_ts")], "TRANSFORMATION", "date(order_ts) [group key]"),
            col("gmv", [("silver.orders_enriched", "gmv")], "AGGREGATION", "sum(gmv)"),
            col("orders", [("silver.orders_enriched", "order_id")], "AGGREGATION", "count()"),
            col("revenue", [("silver.orders_enriched", "revenue_to_caspers")], "AGGREGATION", "sum(revenue_to_caspers)"),
            col("total_margin", [("silver.orders_enriched", "contribution_margin")], "AGGREGATION", "sum(contribution_margin)"),
            col("on_time_rate", [("silver.deliveries_conformed", "is_late")], "AGGREGATION", "1 - mean(is_late)"),
            col("take_rate", [("silver.orders_enriched", "revenue_to_caspers"), ("silver.orders_enriched", "gmv")], "TRANSFORMATION", "sum(revenue) / sum(gmv)"),
        ],
    },
    # ---- ML: features -> forecast -> agent actions --------------------------------
    "ml.demand_forecast": {
        "inputs": ["bronze.order_items", "bronze.orders", "ml.seasonality"],
        "desc": "Forecast per-zone per-ingredient demand from historical order items.",
        "columns": [
            col("zone_id", [("bronze.orders", "zone_id")], "IDENTITY", "join order_items->orders on order_id [group key]"),
            col("ingredient_key", [("bronze.order_items", "ingredient_key")], "IDENTITY", "group key"),
            col("predicted_orders", [("bronze.order_items", "order_id"), ("ml.seasonality", "season_factor")], "TRANSFORMATION", "mean(hist orders) * dow_factor + noise"),
            col("predicted_qty", [("bronze.order_items", "qty"), ("ml.seasonality", "season_factor")], "TRANSFORMATION", "mean(hist qty) * dow_factor + noise"),
            col("lower_90", [("bronze.order_items", "qty")], "TRANSFORMATION", "predicted_qty - band"),
            col("upper_90", [("bronze.order_items", "qty")], "TRANSFORMATION", "predicted_qty + band"),
        ],
    },
    "ml.agent_actions": {
        "inputs": ["ml.demand_forecast", "silver.customers_clean"],
        "desc": "Agent picks top-demand forecasts and proposes actions (reads PII to act).",
        "columns": [
            col("target_id", [("ml.demand_forecast", "ingredient_key")], "IDENTITY", "top-N by predicted_qty"),
            col("zone_id", [("ml.demand_forecast", "zone_id")], "IDENTITY", ""),
            col("predicted_value", [("ml.demand_forecast", "predicted_qty")], "TRANSFORMATION", "predicted_qty * uniform(1.5, 4.0)"),
            col("rationale", [("ml.demand_forecast", "predicted_qty"), ("silver.customers_clean", "zone_id")], "TRANSFORMATION", "templated from forecast + affected customers"),
        ],
    },
}


def build_column_lineage(mappings: list) -> dict:
    """Turn the declarative column mappings into an OpenLineage columnLineage facet."""
    fields = {}
    for out_col, srcs, subtype, desc, masking in mappings:
        kind = "DIRECT" if subtype in ("IDENTITY", "TRANSFORMATION", "AGGREGATION") else "INDIRECT"
        fields[out_col] = {
            "inputFields": [
                {
                    "namespace": NAMESPACE,
                    "name": fq(src_table),
                    "field": src_col,
                    "transformations": [
                        {
                            "type": kind,
                            "subtype": subtype,
                            "description": desc,
                            "masking": masking,
                        }
                    ],
                }
                for src_table, src_col in srcs
            ]
        }
    return {"fields": fields}


def schema_facet(columns: list[str]) -> dict:
    return {
        "_producer": OL_PRODUCER,
        "_schemaURL": SCHEMA_FACET_URL,
        "fields": [{"name": c, "type": "unknown"} for c in columns],
    }


def schema_fields_for(table: str, frames: dict[str, pl.DataFrame]) -> list[dict]:
    """Real columns+types for `table` from the generated frames, else []."""
    df = frames.get(fq(table))
    if df is None:
        return []
    return [{"name": c, "type": str(d)} for c, d in df.schema.items()]


def post(events: list[dict]) -> None:
    body = json.dumps(events).encode()
    req = urllib.request.Request(
        f"{LINEAGE_API}/lineage/batch",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        print(f"  -> {resp.status} {resp.read().decode()[:200]}")


def main() -> int:
    print(f"lineage API: {LINEAGE_API}")
    print(f"namespace:   {NAMESPACE} | catalog: {CATALOG}")

    # Real frames give us accurate input/output column schemas for the facets.
    frames = caspers_gen.generate_all(seed=SEED)
    if CATALOG != "caspers":
        frames = {f"{CATALOG}.{k.split('.', 1)[1]}": v for k, v in frames.items()}

    events: list[dict] = []
    for out_table, spec in DERIVATIONS.items():
        now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%fZ")
        run_id = str(uuid.uuid4())
        job_name = f"transform_{out_table.replace('.', '_')}"

        # Inputs carry their real schemas so the upstream nodes are fully described.
        inputs = [
            {"namespace": NAMESPACE, "name": fq(t), "facets": {"schema": {"_producer": OL_PRODUCER, "_schemaURL": SCHEMA_FACET_URL, "fields": schema_fields_for(t, frames)}}}
            for t in spec["inputs"]
        ]
        out_fields = schema_fields_for(out_table, frames)
        output = {
            "namespace": NAMESPACE,
            "name": fq(out_table),
            "facets": {
                "schema": {"_producer": OL_PRODUCER, "_schemaURL": SCHEMA_FACET_URL, "fields": out_fields},
                "columnLineage": build_column_lineage(spec["columns"]),
            },
        }
        job = {
            "namespace": NAMESPACE,
            "name": job_name,
            "facets": {
                "documentation": {
                    "_producer": OL_PRODUCER,
                    "_schemaURL": "https://openlineage.io/spec/2-0-2/DocumentationJobFacet.json",
                    "description": spec["desc"],
                }
            },
        }
        for ev_type in ("START", "COMPLETE"):
            events.append(
                {
                    "eventType": ev_type,
                    "eventTime": now,
                    "run": {"runId": run_id},
                    "job": job,
                    "inputs": inputs,
                    "outputs": [output],
                    "producer": OL_PRODUCER,
                    "schemaURL": OL_SCHEMA_URL,
                }
            )
        n_edges = sum(len(c[1]) for c in spec["columns"])
        print(f"  {out_table}: {len(spec['inputs'])} inputs, {len(spec['columns'])} cols, {n_edges} column edges")

    print(f"\nposting {len(events)} events ({len(DERIVATIONS)} transforms x START+COMPLETE)...")
    post(events)
    print("done.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
