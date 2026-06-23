// Starter marimo notebook templates, one per query engine.
//
// `notebookTemplate(engine)` returns valid marimo notebook source: a module with
// a PEP 723 `# /// script` dependency block, `import marimo`, an
// `app = marimo.App()`, `@app.cell` functions, and the
// `if __name__ == "__main__": app.run()` tail. The desktop host writes this to a
// `.py` file and opens it; marimo then parses the cells.
//
// The connection setup reads the env vars the desktop notebook sidecar injects
// (see node/desktop/src-tauri/src/notebook.rs): `UC_URI` /
// `OPEN_LAKEHOUSE_UC_URL` (the Unity Catalog REST base, always present),
// `LINEAGE_URL` (the OpenLineage sink base, present only when the environment
// carries the lineage capability), and optionally `UC_TOKEN` (absent today — the
// desktop UC sidecar is unauthenticated).
//
// The Spark template wires UC via the UCSingleCatalog connector and the
// OpenLineage listener conditionally on `LINEAGE_URL`, mirroring the proven
// config in notebooks/spark_lineage.py. The DuckDB and Polars templates build a
// UC-vended `obstore` store (via the sibling unitycatalog-rs Python bindings,
// `unitycatalog_client.obstore`); marimo's Files panel auto-discovers that
// `store` variable and renders a browsable remote source, and cells can read
// data straight from the UC volume.
//
// The PEP 723 block makes the notebook self-describing and portable (Docker /
// marimo-cloud / copied elsewhere). On the desktop the imports are actually
// satisfied by the shared `uvx` environment marimo runs in (which has
// `unitycatalog-client[obstore]` + `obstore`), not by PEP 723 resolution.
//
// These are pure strings: this module imports nothing from Tauri, keeping the
// node/ui Tauri-free seam intact.

export type NotebookEngine = "spark" | "duckdb" | "polars";

/** Human labels for the engine picker. */
export const ENGINE_LABELS: Record<NotebookEngine, string> = {
  spark: "Spark",
  duckdb: "DuckDB",
  polars: "Polars",
};

/** A PEP 723 `# /// script` inline-dependency block. */
function scriptHeader(deps: string[]): string {
  const lines = deps.map((d) => `#     "${d}",`).join("\n");
  return `# /// script
# requires-python = ">=3.10"
# dependencies = [
${lines}
# ]
# ///`;
}

/** A marimo notebook header: PEP 723 block + the marimo app object. */
function header(deps: string[]): string {
  return `${scriptHeader(deps)}

import marimo

app = marimo.App(width="medium")
`;
}

const FOOTER = `
if __name__ == "__main__":
    app.run()
`;

// The desktop sidecar injects UC_URI / OPEN_LAKEHOUSE_UC_URL as the full REST
// base (".../api/2.1/unity-catalog/"). `UC_REST` keeps that base verbatim (the
// obstore credential client wants it), while `UC_URI` is stripped to the server
// root (the UCSingleCatalog connector + plain REST helpers want that).
const UC_ENV_CELL = `@app.cell
def _():
    import os

    # The desktop notebook host injects the Unity Catalog REST base under both
    # names; LINEAGE_URL is present only when the environment runs a lineage
    # service; UC_TOKEN is absent today (the desktop UC sidecar is unauthenticated).
    _uc_rest = os.environ.get("UC_URI") or os.environ.get("OPEN_LAKEHOUSE_UC_URL") or ""
    # obstore's credential client wants the full REST base (trailing slash);
    UC_REST = _uc_rest if _uc_rest.endswith("/") else _uc_rest + "/"
    # the Spark connector + plain REST helpers want the server root instead.
    UC_URI = _uc_rest.rstrip("/").removesuffix("/api/2.1/unity-catalog")
    UC_TOKEN = os.environ.get("UC_TOKEN") or None
    LINEAGE_URL = os.environ.get("LINEAGE_URL")
    CATALOG = "main"
    return CATALOG, LINEAGE_URL, UC_REST, UC_TOKEN, UC_URI
`;

// A UC-vended obstore store rooted at a volume. The `store` variable is what
// marimo's Files panel auto-discovers as a browsable remote source. Shared by
// the DuckDB + Polars templates.
const UC_STORE_CELL = `@app.cell
def _(CATALOG, UC_REST, UC_TOKEN):
    from unitycatalog_client import TemporaryCredentialClient
    from unitycatalog_client.obstore import store_for_volume

    # Unity Catalog vends temporary cloud credentials and refreshes them
    # automatically. marimo's Files panel auto-discovers this \`store\` and renders
    # a remote browser for it. Point it at your data by changing the volume name
    # ("catalog.schema.volume"); pass operation="read_write" to enable writes.
    _client = TemporaryCredentialClient(base_url=UC_REST, token=UC_TOKEN)
    store = store_for_volume(_client, f"{CATALOG}.default.landing", operation="read")
    return (store,)
`;

// PEP 723 dependency sets per engine. Spark pins to match the baked jars (see
// environments/docker/marimo/pyproject.toml); duckdb/polars pull the UC client +
// obstore for the credential-vended store.
const DEPS: Record<NotebookEngine, string[]> = {
  spark: ["marimo", "pyspark==4.1.2", "delta-spark==4.1.0", "requests"],
  duckdb: [
    "marimo",
    "duckdb",
    "pyarrow",
    "unitycatalog-client[obstore]",
    "obstore>=0.5",
  ],
  polars: [
    "marimo",
    "polars",
    "pyarrow",
    "unitycatalog-client[obstore]",
    "obstore>=0.5",
  ],
};

function sparkTemplate(): string {
  // Ported from notebooks/spark_lineage.py: Delta + UCSingleCatalog, optional
  // OpenLineage listener. Token is empty (the desktop UC sidecar is
  // unauthenticated today). Spark uses the connector, not obstore.
  const sparkCell = `@app.cell
def _(CATALOG, LINEAGE_URL, UC_URI):
    import pyspark

    # Delta + Unity Catalog. UC OSS dev: empty token. We do not set S3 creds —
    # UC vends temporary credentials and the connector injects them per table.
    _builder = (
        pyspark.sql.SparkSession.builder.appName("notebook")
        .config("spark.sql.extensions", "io.delta.sql.DeltaSparkSessionExtension")
        .config("spark.sql.catalog.spark_catalog", "io.unitycatalog.spark.UCSingleCatalog")
        .config(f"spark.sql.catalog.{CATALOG}", "io.unitycatalog.spark.UCSingleCatalog")
        .config(f"spark.sql.catalog.{CATALOG}.uri", UC_URI)
        .config(f"spark.sql.catalog.{CATALOG}.token", "")
        .config("spark.sql.defaultCatalog", CATALOG)
    )

    # Wire the OpenLineage listener only when a lineage service is running.
    if LINEAGE_URL:
        _builder = (
            _builder
            .config("spark.extraListeners", "io.openlineage.spark.agent.OpenLineageSparkListener")
            .config("spark.openlineage.transport.type", "http")
            .config("spark.openlineage.transport.url", LINEAGE_URL)
            .config("spark.openlineage.transport.endpoint", "/api/v1/lineage")
            .config("spark.openlineage.namespace", "notebook")
            .config("spark.openlineage.columnLineage.datasetLineageEnabled", "true")
        )

    spark = _builder.getOrCreate()
    spark.sparkContext.setLogLevel("WARN")
    return (spark,)
`;

  const queryCell = `@app.cell
def _(spark):
    # Your queries go here. Example:
    # spark.sql("SHOW CATALOGS").show()
    return
`;

  return `${header(DEPS.spark)}

${UC_ENV_CELL}

${sparkCell}

${queryCell}
${FOOTER}`;
}

function duckdbTemplate(): string {
  const cell = `@app.cell
def _(store):
    import duckdb
    import obstore as obs
    import pyarrow.parquet as pq

    con = duckdb.connect()
    # Example — read one parquet object from the UC volume via obstore, then query it:
    # table = pq.read_table(obs.open_reader(store, "path/to/file.parquet"))
    # con.register("t", table)
    # con.sql("SELECT * FROM t LIMIT 10")
    return (con,)
`;
  return `${header(DEPS.duckdb)}

${UC_ENV_CELL}

${UC_STORE_CELL}

${cell}
${FOOTER}`;
}

function polarsTemplate(): string {
  const cell = `@app.cell
def _(store):
    import obstore as obs
    import polars as pl

    # Example — read a parquet object straight from the UC volume via obstore:
    # df = pl.read_parquet(obs.open_reader(store, "path/to/file.parquet"))
    # df.head()
    return (pl,)
`;
  return `${header(DEPS.polars)}

${UC_ENV_CELL}

${UC_STORE_CELL}

${cell}
${FOOTER}`;
}

/** Build starter notebook source for the chosen engine. */
export function notebookTemplate(engine: NotebookEngine): string {
  switch (engine) {
    case "spark":
      return sparkTemplate();
    case "duckdb":
      return duckdbTemplate();
    case "polars":
      return polarsTemplate();
  }
}
