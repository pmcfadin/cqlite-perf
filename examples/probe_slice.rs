//! Throwaway probe (issue #11): which clustering-slice query forms does
//! cqlite-core v0.9.2 actually return rows for? Ingests the wide_rows corpus
//! and prints row counts for several WHERE shapes. Not part of the harness.
use cqlite_core::ingestion::{ingest, IngestionConfig};
use cqlite_core::query::result::StreamingConfig;
use cqlite_core::{Config, Database};

async fn count(db: &Database, q: &str) -> String {
    match db.execute_streaming(q, StreamingConfig::default()).await {
        Ok(mut it) => {
            let mut n = 0u64;
            let mut err = None;
            while let Some(row) = it.next_async().await {
                match row {
                    Ok(_) => n += 1,
                    Err(e) => {
                        err = Some(format!("{e}"));
                        break;
                    }
                }
            }
            match err {
                Some(e) => format!("ERR after {n}: {e}"),
                None => format!("{n} rows"),
            }
        }
        Err(e) => format!("EXEC ERR: {e}"),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let cfg = IngestionConfig {
        schema_paths: vec![root.join("schemas/wide_rows.cql")],
        data_dir: root.join("datasets/read-wide_rows-S-lz4"),
        version_hint: Some("5.0".to_string()),
        core_config: Config::default(),
        table_directory_filter: None,
    };
    let db = ingest(cfg).await.map_err(|e| anyhow::anyhow!("ingest: {e}"))?.database;

    for q in [
        "SELECT * FROM wide_rows",
        "SELECT * FROM wide_rows WHERE pk = 'p0000'",
        "SELECT * FROM wide_rows WHERE pk = 'p0000' AND ck >= 0 AND ck < 200",
        "SELECT * FROM wide_rows WHERE pk = 'p0000' AND ck BETWEEN 0 AND 199",
        "SELECT * FROM wide_rows WHERE pk = 'p0000' AND ck < 200",
        "SELECT * FROM wide_rows WHERE pk = 'p0000' AND ck >= 800",
    ] {
        println!("{:60} => {}", q, count(&db, q).await);
    }
    Ok(())
}
