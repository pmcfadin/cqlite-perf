//! Throwaway probe (issue #11): does partition-key equality return rows on the
//! basic corpus (point_lookup), or is the equality path broken across corpora?
use cqlite_core::ingestion::{ingest, IngestionConfig};
use cqlite_core::query::result::StreamingConfig;
use cqlite_core::{Config, Database};

async fn count(db: &Database, q: &str) -> String {
    match db.execute_streaming(q, StreamingConfig::default()).await {
        Ok(mut it) => { let mut n=0u64; while let Some(r)=it.next_async().await { if r.is_ok(){n+=1} else {return format!("ERR after {n}")} } format!("{n} rows") }
        Err(e) => format!("EXEC ERR: {e}"),
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
        "SELECT * FROM basic",
        "SELECT * FROM basic WHERE id = 'k0000000000000000'",
        "SELECT * FROM basic WHERE id = 'k0000000000000001'",
        "SELECT * FROM basic LIMIT 3",
    ] { println!("{:55} => {}", q, count(&db, q).await); }
    Ok(())
}
