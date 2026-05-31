//! Read suite (SPEC §6), atop the Cassandra-generated canonical corpus
//! (SPEC §8.1). All read workloads share one engine: open the dataset once via
//! `ingestion::ingest`, then each `op()` runs a SQL query and counts rows.
//!
//! M1 ships `read.full_scan` (streaming `SELECT *`). The point/slice/wide/
//! type-heavy variants are the same machinery with a different query and key
//! selection; they land incrementally on top of this.

use std::sync::Arc;

use async_trait::async_trait;
use cqlite_core::ingestion::{ingest, IngestionConfig};
use cqlite_core::query::result::StreamingConfig;
use cqlite_core::{Config, Database};

use super::{OpRows, RunContext, Workload};

/// Repo-root-relative manifests directory.
const MANIFESTS_DIR: &str = "datasets/manifests";

/// A read workload parameterized by the SQL it issues. One open DB shared by all
/// workers (reads are `&self` on `Database`).
pub struct ReadWorkload {
    name: &'static str,
    /// The query each `op()` executes.
    query: String,
    db: Option<Arc<Database>>,
}

impl ReadWorkload {
    /// `read.full_scan` — streaming `SELECT *` over the whole table. Raw decode
    /// throughput → the rows/sec headline (SPEC §6).
    pub fn full_scan(table: &str) -> Self {
        Self {
            name: "read.full_scan",
            query: format!("SELECT * FROM {table}"),
            db: None,
        }
    }
}

#[async_trait]
impl Workload for ReadWorkload {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn setup(&mut self, ctx: &RunContext) -> anyhow::Result<()> {
        // Resolve the dataset by manifest query (tier+schema+codec), verify its
        // checksum, then open it via the one-shot ingestion flow.
        let repo_root = std::env::current_dir()?;
        let manifests_dir = repo_root.join(MANIFESTS_DIR);
        let q = crate::datasets::ManifestQuery {
            tier: ctx.tier.clone(),
            schema: ctx.schema.clone(),
            codec: ctx.codec.clone(),
        };
        let manifest = crate::datasets::resolve(&manifests_dir, &q)?;
        let dataset_dir = crate::datasets::data_dir(&repo_root, &manifest);

        let computed = crate::datasets::compute_data_sha256(&dataset_dir)?;
        if computed != manifest.sha256 {
            anyhow::bail!(
                "checksum mismatch for {}: manifest {} != computed {} (regenerate the dataset)",
                manifest.id,
                manifest.sha256,
                computed
            );
        }

        let schema_path = repo_root.join(&manifest.cqlite_schema_ref);
        let cfg = IngestionConfig {
            schema_paths: vec![schema_path],
            data_dir: dataset_dir,
            version_hint: Some("5.0".to_string()),
            core_config: Config::default(),
            table_directory_filter: None,
        };

        let result = ingest(cfg)
            .await
            .map_err(|e| anyhow::anyhow!("ingest dataset {}: {e}", manifest.id))?;
        self.db = Some(Arc::new(result.database));
        Ok(())
    }

    async fn op(&self, _worker: usize) -> anyhow::Result<OpRows> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("{}: setup() not called", self.name))?;

        // Stream rows through a bounded channel so a full scan stays memory-
        // bounded (SPEC §7 <128 MB target for streaming reads).
        let mut iter = db
            .execute_streaming(&self.query, StreamingConfig::default())
            .await
            .map_err(|e| anyhow::anyhow!("execute_streaming: {e}"))?;

        let mut rows: u64 = 0;
        while let Some(row) = iter.next_async().await {
            row.map_err(|e| anyhow::anyhow!("row decode: {e}"))?;
            rows += 1;
        }
        Ok(rows)
    }

    async fn teardown(&mut self) -> anyhow::Result<()> {
        if let Some(db) = self.db.take() {
            if let Ok(db) = Arc::try_unwrap(db) {
                db.shutdown()
                    .await
                    .map_err(|e| anyhow::anyhow!("Database::shutdown: {e}"))?;
            }
        }
        Ok(())
    }
}
