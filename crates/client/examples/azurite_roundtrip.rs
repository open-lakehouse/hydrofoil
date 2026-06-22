//! End-to-end managed Delta table round-trip **through a live hydrofoil server**,
//! exercising the Azurite (Azure Blob emulator) credential-vending path.
//!
//! Unlike the in-process integration tests, this drives a real hydrofoil over the
//! network: create a managed table, ingest Arrow batches (ADBC-style
//! `do_put_statement_ingest`), then read them back via Flight SQL. The data lands
//! in Azurite via the SAS hydrofoil vends from Unity Catalog — so a successful
//! run proves the whole vend → write → read path on the emulator.
//!
//! ## Running
//!
//! ```bash
//! # 1. Azurite:            just env-up azurite
//! # 2. Rust UC server:     cargo run -p unitycatalog-cli -- server --rest --port 8081 \
//! #                          --config environments/config/azurite/uc-config.yaml   (sibling repo)
//! # 3. Seed credential:    ./scripts/azurite-seed.sh
//! # 4. Create catalog/schema (managed root inherited from the UC config):
//! #      curl -XPOST .../catalogs -d '{"name":"azc"}'
//! #      curl -XPOST .../schemas  -d '{"name":"s","catalog_name":"azc"}'
//! # 5. hydrofoil:          HYDROFOIL_CONFIG=environments/config/azurite/hydrofoil.toml just hydro
//! # 6. This example:
//! HYDROFOIL_URL=http://localhost:50051 UC_TABLE=azc.s.orders_hf \
//!   cargo run -p hydrofoil-client --example azurite_roundtrip
//! ```

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use datafusion_common::TableReference;
use futures::TryStreamExt;
use hydrofoil_client::Client;

type BoxError = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let url =
        std::env::var("HYDROFOIL_URL").unwrap_or_else(|_| "http://localhost:50051".to_string());
    // Fully-qualified managed table name (catalog.schema.table). The catalog +
    // schema must already exist in UC; the managed catalog inherits the
    // azurite:// managed_storage_root from the server config.
    let fq = std::env::var("UC_TABLE").unwrap_or_else(|_| "azc.s.orders_hf".to_string());
    let table_ref = TableReference::parse_str(&fq);

    let arrow_schema = Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("item", DataType::Utf8, true),
    ]);

    let mut client = Client::try_new(url.clone()).await?;
    println!("connected to hydrofoil at {url}");

    // ── Create the managed table (empty location ⇒ UC allocates it on Azurite) ──
    // Set SKIP_CREATE=1 when the table was created out-of-band (e.g. via the
    // datafusion-unitycatalog `managed_table_azurite` example) — hydrofoil's
    // CREATE path needs the catalog registered in the session, whereas the
    // ingest/read paths resolve the table directly through UC.
    match std::env::var("CREATE_MODE").as_deref() {
        Ok("sql") => {
            // SQL CREATE TABLE … USING DELTA goes through hydrofoil's SQL planner,
            // which resolves + registers the UC catalog before planning. Minimal
            // form only: no TBLPROPERTIES (the server negotiates catalogManaged).
            let ddl = format!("CREATE TABLE {fq} (id BIGINT, item STRING) USING DELTA");
            println!("creating via SQL: {ddl}");
            let _ = client
                .execute(ddl, None)
                .await?
                .try_collect::<Vec<_>>()
                .await?;
            println!("  created");
        }
        Ok("skip") => println!("CREATE_MODE=skip — assuming {fq} already exists"),
        _ => {
            // Default: DeltaConnect do_put create (note: this path does not
            // register the UC catalog in the session — see README/notes).
            println!("creating via DeltaConnect do_put: {fq} …");
            client
                .create_delta_table()
                .with_table_name(&fq)
                .with_location("")
                .with_schema(&arrow_schema)?
                .await?;
            println!("  created");
        }
    }

    // ── Ingest rows (do_put_statement_ingest → append_to_managed_table → Azurite SAS) ──
    let batch = RecordBatch::try_new(
        Arc::new(arrow_schema.clone()),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec!["alpha", "beta", "gamma"])),
        ],
    )?;
    let input = futures::stream::once(async move { Ok(batch) });
    let rows = client.ingest(table_ref, input).await?;
    println!("ingested {rows} rows");

    // ── Read them back via Flight SQL (do_get_statement → unity resolver → vended store) ──
    let stream = client
        .execute(format!("SELECT * FROM {fq} ORDER BY id"), None)
        .await?;
    let batches: Vec<RecordBatch> = stream.try_collect().await?;
    let total: usize = batches.iter().map(|b| b.num_rows()).sum();
    println!("read back {total} rows");
    for b in &batches {
        let ids = b
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .map(|a| a.iter().flatten().collect::<Vec<_>>());
        println!("  ids: {ids:?}");
    }

    // The ingest writes 3 rows; the table may already contain rows from an
    // out-of-band create+append (SKIP_CREATE path), so assert we wrote 3 and can
    // read back at least the 3 we just wrote.
    if rows != 3 || total < 3 {
        return Err(format!(
            "expected 3 rows written and ≥3 read, got written={rows} read={total}"
        )
        .into());
    }
    println!("\nSUCCESS: managed Delta table write + read through hydrofoil works on Azurite.");
    Ok(())
}
