//! What columns/values does a scanned row actually expose? (issue #13 root cause)
use cqlite_core::ingestion::{ingest, IngestionConfig};
use cqlite_core::query::result::StreamingConfig;
use cqlite_core::{Config, Database};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let cfg = IngestionConfig {
        schema_paths: vec![root.join("schemas/basic.cql")],
        data_dir: root.join("datasets/read-basic-S-lz4"),
        version_hint: Some("5.0".to_string()),
        core_config: Config::default(),
        table_directory_filter: None,
    };
    let db = ingest(cfg).await.map_err(|e| anyhow::anyhow!("ingest: {e}"))?.database;
    let mut it = db.execute_streaming("SELECT * FROM basic", StreamingConfig::default()).await?;
    let mut shown = 0;
    while let Some(Ok(row)) = it.next_async().await {
        println!("columns: {:?}", row.column_names());
        for c in row.column_names() {
            println!("  {c} = {:?}", row.get(&c));
        }
        println!("  raw key = {:?}", row.key);
        shown += 1;
        if shown >= 2 { break; }
    }
    Ok(())
}
