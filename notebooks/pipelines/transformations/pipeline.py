# Spark Declarative Pipelines graph for the OpenLineage spike.
#
# REQUIRES pyspark 4.1.0+ : the `pyspark.pipelines` module and the `spark-pipelines`
# launcher are absent from pyspark 4.0.1 (verified 2026-06-08) and only appear in
# 4.1.0. Under 4.0.1 the import below fails.
#
# Two materialized views forming a single input -> output edge:
#   raw_events  (seed)  ->  events_by_kind  (aggregate)
# Both materialize as UC-managed Delta tables in demo.sdp_demo (per spark-pipeline.yml).
# Kept intentionally tiny: the point is to see whether SDP flow execution surfaces
# through the OpenLineage Spark listener, not the data itself.

from pyspark import pipelines as dp
from pyspark.sql import DataFrame, SparkSession
from pyspark.sql import functions as F

spark = SparkSession.active()


@dp.materialized_view
def raw_events() -> DataFrame:
    return spark.createDataFrame(
        [
            (1, "login", "2026-06-02 09:00:00"),
            (2, "click", "2026-06-02 09:01:00"),
            (3, "click", "2026-06-02 09:02:00"),
            (4, "logout", "2026-06-02 09:05:00"),
        ],
        "id BIGINT, kind STRING, ts STRING",
    )


@dp.materialized_view
def events_by_kind() -> DataFrame:
    return (
        spark.read.table("raw_events")
        .groupBy("kind")
        .agg(F.count("*").alias("n"), F.max("ts").alias("last_seen"))
    )
