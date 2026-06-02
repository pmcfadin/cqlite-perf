//! v0.10.0 isolation (issue #13): non-PK column filters vs PK column filter.
use cqlite_core::ingestion::{ingest, IngestionConfig};
use cqlite_core::query::result::StreamingConfig;
use cqlite_core::{Config, Database};
async fn count(db: &Database, q: &str) -> i64 {
    match db.execute_streaming(q, StreamingConfig::default()).await {
        Ok(mut it) => { let mut n=0; while let Some(r)=it.next_async().await { if r.is_ok(){n+=1} else {return -1} } n }
        Err(_) => -2,
    }
}
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
    for q in [
        "SELECT * FROM basic WHERE age = 0",                       // non-PK regular col
        "SELECT * FROM basic WHERE name = 'name-0'",               // non-PK regular col
        "SELECT * FROM basic WHERE email = 'u0@perf.test'",        // non-PK regular col
        "SELECT * FROM basic WHERE id = 'k0000000000000000'",      // PK col
    ] { println!("{:55} => {} rows", q, count(&db, q).await); }
    Ok(())
}
