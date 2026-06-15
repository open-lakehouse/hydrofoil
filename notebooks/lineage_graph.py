# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "delta-spark==4.2.0",
#     "pyspark==4.1.1",
#     "marimo",
# ]
# ///

# Lineage events -> flat graph (PuppyGraph-ready).
#
# Our lineage-service writes OpenLineage 2.0.2 events to a 15-column Delta `events`
# table (locally environments/.data/lineage/events). This notebook reshapes those
# events into a FLAT GRAPH that PuppyGraph can query: one Delta table per vertex type
# (with a unique string `id`) and one per edge type (with string `id`/`from_id`/`to_id`).
#
#   Dataset --[CONSUMED_BY]--> Job --[PRODUCES]--> Dataset
#
# PuppyGraph integration (deployed in a SECOND step):
#   - PuppyGraph reads Delta by resolving a table's storage_location from a metastore
#     (Hive/Glue/Unity) and reading the Delta log AT THAT PATH — the EXTERNAL-table
#     model. It does NOT implement the UC catalog-managed (delta.feature.catalogManaged)
#     commit-coordinator read path, so UC-MANAGED tables are unsupported/unverified.
#   - So we target UC EXTERNAL tables for PuppyGraph. Our live env has no external
#     location registered yet, so FOR NOW we write the graph tables as plain Delta to a
#     LOCAL path (mirroring the open-lineage-connect/lineage-graph prior art). Registering
#     them as UC external tables (pointing at the file:// path) is a follow-up once an
#     external location exists. See lineage_graph_schema.json for the PuppyGraph schema.
#
# CDC: the events table has no Change Data Feed today (and CDF through the UC-managed
# connector is unverified), so v1 does a FULL REBUILD (read all events, overwrite the
# graph tables). The CDC-ready upgrade path is documented in the note at the bottom.
#
# Prerequisites:
#   - An events Delta table to read. By default the local path the live stack writes
#     (environments/.data/lineage/events); run notebooks/spark_lineage.py first to
#     populate it, or point LINEAGE_EVENTS_PATH at any events table (incl. s3://...).
#
# Run on the host (from the repo root so the default relative path resolves):
#   uvx --directory notebooks/ marimo edit --sandbox lineage_graph.py

import marimo

__generated_with = "0.23.8"
app = marimo.App(width="medium")


@app.cell
def _():
    import marimo as mo

    return (mo,)


@app.cell
def _(mo):
    mo.md("""
    # Lineage events -> flat graph (PuppyGraph-ready)

    Our lineage service writes OpenLineage events to a Delta **`events`** table. Here
    we reshape them into a **flat graph** — one Delta table per vertex type (unique
    string `id`) and one per edge type (`id` / `from_id` / `to_id`) — the layout
    [PuppyGraph](https://docs.puppygraph.com) maps to a graph:

    ```
    Dataset --[CONSUMED_BY]--> Job --[PRODUCES]--> Dataset
    ```

    **PuppyGraph reads Delta as *external* tables** (it resolves a `storage_location`
    from a metastore and reads the Delta log at that path); it does **not** implement
    the UC *catalog-managed* read path. So we target UC external tables — but since our
    live env has no external location yet, this notebook writes plain Delta to a **local
    path** for now. PuppyGraph deployment + UC external registration is a second step
    (see `lineage_graph_schema.json`).
    """)
    return


@app.cell
def _():
    import os

    # Source events table. Default to the local Delta path the live stack writes
    # (compose project dir is environments/); override via env for an S3 table.
    EVENTS_PATH = os.environ.get(
        "LINEAGE_EVENTS_PATH", "environments/.data/lineage/events"
    )

    # Output root for the flat-graph Delta tables (local path for now).
    GRAPH_OUT = os.environ.get("LINEAGE_GRAPH_OUT", "environments/.data/lineage_graph")

    AWS_REGION = "eu-central-1"  # only used if EVENTS_PATH is on S3

    print("events:", EVENTS_PATH)
    print("graph out:", GRAPH_OUT)
    return AWS_REGION, EVENTS_PATH, GRAPH_OUT


@app.cell
def _(AWS_REGION):
    # A plain Delta session — no OpenLineage listener and no UCSingleCatalog: we read and
    # write Delta tables BY PATH, not through Unity Catalog. S3A/region are kept so the
    # same notebook works against an events table on S3 later.
    import pyspark

    builder = (
        pyspark.sql.SparkSession.builder.appName("lineage-graph")
        .config("spark.sql.extensions", "io.delta.sql.DeltaSparkSessionExtension")
        .config(
            "spark.sql.catalog.spark_catalog",
            "org.apache.spark.sql.delta.catalog.DeltaCatalog",
        )
        .config("spark.hadoop.fs.s3.impl", "org.apache.hadoop.fs.s3a.S3AFileSystem")
        .config("spark.hadoop.fs.s3a.endpoint.region", AWS_REGION)
    )

    # No spark.jars.packages / Ivy / Maven proxy: the Delta and hadoop-aws jars are baked
    # onto pyspark's classpath by the marimo image.
    spark = builder.getOrCreate()
    spark.sparkContext.setLogLevel("WARN")
    return (spark,)


@app.cell
def _(EVENTS_PATH, spark):
    from pyspark.sql.functions import from_json, explode, col, sha2, concat_ws, lit
    from pyspark.sql.types import ArrayType, StructType, StructField, StringType
    from pyspark.sql.window import Window

    # OpenLineage inputs/outputs are JSON arrays of {name, namespace} (the events table
    # stores them as the inputs_json / outputs_json string columns).
    dataset_schema = ArrayType(
        StructType(
            [
                StructField("name", StringType()),
                StructField("namespace", StringType()),
            ]
        )
    )

    # Read all RUN events (job/dataset events carry no run<->dataset edges).
    events = spark.read.format("delta").load(EVENTS_PATH).where(col("event_kind") == "run")

    parsed = events.withColumn(
        "inputs", from_json(col("inputs_json"), dataset_schema)
    ).withColumn("outputs", from_json(col("outputs_json"), dataset_schema))

    print("run events:", events.count())
    events.select(
        "run_id", "job_namespace", "job_name", "event_type", "event_time"
    ).show(10, truncate=False)
    return Window, col, concat_ws, explode, lit, parsed, sha2


@app.cell
def _(Window, col, parsed):
    from pyspark.sql.functions import row_number

    # v_job: one vertex per run_id. A run emits START + COMPLETE (and sometimes more);
    # keep the latest event per run_id (COMPLETE wins on ties via event_type) so each run
    # collapses to a single vertex carrying its terminal state.
    _w = Window.partitionBy("run_id").orderBy(
        col("event_time").desc(), col("event_type").desc()
    )
    v_job = (
        parsed.where(col("run_id").isNotNull())
        .withColumn("_rn", row_number().over(_w))
        .where(col("_rn") == 1)
        .select(
            col("run_id").alias("id"),
            "job_name",
            "job_namespace",
            "event_type",
            "event_time",
            "producer",
        )
    )
    v_job.show(10, truncate=False)
    return (v_job,)


@app.cell
def _(Window, col, concat_ws, explode, lit, parsed, sha2):
    from pyspark.sql.functions import row_number

    # Explode inputs/outputs into (run_id, dataset) rows. A dataset's identity is
    # (namespace, name); its vertex id is a deterministic string hash of that pair so the
    # same dataset referenced as an input here and an output there maps to one vertex.
    def _dataset_id(ns, name):
        return sha2(concat_ws("", ns, name), 256)

    def _explode(arr_col):
        return (
            parsed.where(col(arr_col).isNotNull())
            .select(
                col("run_id"),
                "event_time",
                "event_type",
                explode(arr_col).alias("ds"),
            )
            .select(
                "run_id",
                "event_time",
                "event_type",
                col("ds.name").alias("dataset_name"),
                col("ds.namespace").alias("dataset_namespace"),
            )
            .withColumn(
                "dataset_id", _dataset_id(col("dataset_namespace"), col("dataset_name"))
            )
        )

    inputs = _explode("inputs")
    outputs = _explode("outputs")

    # Collapse to ONE edge per (from, to): a run emits the same edge on START and COMPLETE
    # (differing only in event_time/type). Keep the latest event so the edge id — a hash of
    # the endpoint pair plus direction — stays unique (PuppyGraph requires unique edge ids).
    def _edges(rows, from_col, to_col, direction):
        w = Window.partitionBy("from_id", "to_id").orderBy(
            col("event_time").desc(), col("event_type").desc()
        )
        return (
            rows.select(
                col(from_col).alias("from_id"),
                col(to_col).alias("to_id"),
                "event_time",
                "event_type",
            )
            .withColumn("_rn", row_number().over(w))
            .where(col("_rn") == 1)
            .select(
                sha2(concat_ws("", col("from_id"), col("to_id"), lit(direction)), 256).alias("id"),
                "from_id",
                "to_id",
                "event_time",
                "event_type",
            )
        )

    # v_dataset: distinct datasets across inputs and outputs.
    v_dataset = (
        inputs.select("dataset_id", "dataset_name", "dataset_namespace")
        .union(outputs.select("dataset_id", "dataset_name", "dataset_namespace"))
        .distinct()
        .select(col("dataset_id").alias("id"), "dataset_name", "dataset_namespace")
    )

    # e_consumed_by: Dataset -> Job (an input was consumed by the run).
    e_consumed_by = _edges(inputs, "dataset_id", "run_id", "in")
    # e_produces: Job -> Dataset (the run produced an output).
    e_produces = _edges(outputs, "run_id", "dataset_id", "out")

    print("datasets:", v_dataset.count())
    print("consumed_by edges:", e_consumed_by.count())
    print("produces edges:", e_produces.count())
    return e_consumed_by, e_produces, v_dataset


@app.cell
def _(GRAPH_OUT, e_consumed_by, e_produces, v_dataset, v_job):
    # Full rebuild: overwrite each table. PuppyGraph vertex/edge tables -> one Delta dir each.
    tables = {
        "v_job": v_job,
        "v_dataset": v_dataset,
        "e_consumed_by": e_consumed_by,
        "e_produces": e_produces,
    }
    for _name, _df in tables.items():
        _df.write.format("delta").mode("overwrite").option(
            "overwriteSchema", "true"
        ).save(f"{GRAPH_OUT}/{_name}")
        print("wrote", f"{GRAPH_OUT}/{_name}")
    return (tables,)


@app.cell
def _(GRAPH_OUT, spark, tables):
    # Verify: ids unique per vertex/edge table, and every edge endpoint resolves to a vertex.
    def _load(name):
        return spark.read.format("delta").load(f"{GRAPH_OUT}/{name}")

    vj, vd = _load("v_job"), _load("v_dataset")
    ec, ep = _load("e_consumed_by"), _load("e_produces")

    for _name in tables:
        _df = _load(_name)
        _dupes = _df.count() - _df.select("id").distinct().count()
        print(f"{_name}: {_df.count()} rows, duplicate ids: {_dupes}")
        assert _dupes == 0, f"{_name} has duplicate ids"

    # Dangling-endpoint checks (anti-join count must be 0).
    dangling = {
        "consumed_by.from->dataset": ec.join(vd, ec.from_id == vd.id, "left_anti").count(),
        "consumed_by.to->job": ec.join(vj, ec.to_id == vj.id, "left_anti").count(),
        "produces.from->job": ep.join(vj, ep.from_id == vj.id, "left_anti").count(),
        "produces.to->dataset": ep.join(vd, ep.to_id == vd.id, "left_anti").count(),
    }
    print("dangling edge endpoints:", dangling)
    assert all(v == 0 for v in dangling.values()), f"dangling endpoints: {dangling}"
    return


@app.cell
def _(mo):
    mo.md("""
    ## CDC-ready upgrade path (not implemented in v1)

    v1 does a **full rebuild** — robust and matches the prior-art demo. To process
    **incrementally** once the events table enables Change Data Feed
    (`ALTER TABLE … SET TBLPROPERTIES ('delta.enableChangeDataFeed' = 'true')`):

    1. Read only new commits:
       ```python
       changes = (
           spark.read.format("delta")
           .option("readChangeFeed", "true")
           .option("startingVersion", last_processed_version)
           .load(EVENTS_PATH)
       )
       ```
    2. Run the **same** explode/hash transforms over `changes` to derive new vertices/edges.
    3. `MERGE INTO` each graph table on `id` (vertex ids and edge ids are deterministic, so
       re-seen rows are idempotent updates, not duplicates). Persist the processed version.

    Caveat: CDF **through the UC catalog-managed connector** is currently unverified — this
    path is designed for, not yet exercised.

    ## Column-level lineage (TODO)

    The events table also carries `column_lineage_json` (per-column input→output field
    mappings). A future `e_field_lineage` (Dataset→Dataset / field→field) edge table could be
    derived from it. Deferred: on Spark 4.0 the OpenLineage listener emits **sparse**
    dataset/columnLineage facets for UC catalog-managed tables (see `spark_lineage.py`), so
    we avoid shipping empty edges until richer facets are available.
    """)
    return


if __name__ == "__main__":
    app.run()
