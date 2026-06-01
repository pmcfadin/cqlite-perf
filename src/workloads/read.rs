//! Read suite (SPEC §6), atop the Cassandra-generated canonical corpus
//! (SPEC §8.1). All read workloads share one engine: open the dataset once via
//! `ingestion::ingest`, then each `op()` runs a SQL query and counts rows.
//!
//! M1 ships `read.full_scan` (streaming `SELECT *`). The point/slice/wide/
//! type-heavy variants are the same machinery with a different query and key
//! selection; they land incrementally on top of this.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use cqlite_core::ingestion::{ingest, IngestionConfig};
use cqlite_core::query::result::StreamingConfig;
use cqlite_core::{Config, Database};

use super::{DatasetMeta, OpRows, RunContext, Workload};
use crate::distribution::KeyGen;

/// Repo-root-relative manifests directory.
const MANIFESTS_DIR: &str = "datasets/manifests";

/// How a workload forms the query for each `op()`.
enum QueryMode {
    /// One fixed query (scan-style).
    Fixed(String),
    /// A point lookup: `SELECT * FROM <table> WHERE <pk_col> = '<key>'`, with the
    /// key drawn per-op from a precomputed seeded sequence (SPEC §6).
    Point {
        table: String,
        pk_col: String,
        /// Precomputed keys in distribution+seed order; `op()` cycles by index.
        keys: Vec<String>,
        cursor: AtomicUsize,
    },
}

/// A read workload parameterized by the SQL it issues. One open DB shared by all
/// workers (reads are `&self` on `Database`).
pub struct ReadWorkload {
    name: &'static str,
    mode: QueryMode,
    db: Option<Arc<Database>>,
    /// On-disk size + row count of the resolved corpus, set at `setup`.
    meta: Option<DatasetMeta>,
}

impl ReadWorkload {
    /// `read.full_scan` — streaming `SELECT *` over the whole table. Raw decode
    /// throughput → the rows/sec headline (SPEC §6).
    pub fn full_scan(table: &str) -> Self {
        Self {
            name: "read.full_scan",
            mode: QueryMode::Fixed(format!("SELECT * FROM {table}")),
            db: None,
            meta: None,
        }
    }

    /// `read.type_heavy` — scan over a collections/UDT shape. Stresses
    /// deserialization cost vs. I/O (SPEC §6). Same scan shape, richer schema.
    pub fn type_heavy(table: &str) -> Self {
        Self {
            name: "read.type_heavy",
            mode: QueryMode::Fixed(format!("SELECT * FROM {table}")),
            db: None,
            meta: None,
        }
    }

    /// `read.wide_partition` — full scan over a `wide_rows` shape, exercising
    /// large-partition behavior (SPEC §6).
    pub fn wide_partition(table: &str) -> Self {
        Self {
            name: "read.wide_partition",
            mode: QueryMode::Fixed(format!("SELECT * FROM {table}")),
            db: None,
            meta: None,
        }
    }

    /// `read.point_lookup` — `SELECT … WHERE pk = ?` against a single partition.
    /// Latency-sensitive; stresses Index.db/Summary.db + partition seek (SPEC §6).
    /// Keys match the `basic` corpus key format (`k{i:016x}`).
    pub fn point_lookup(table: &str, pk_col: &str) -> Self {
        Self {
            name: "read.point_lookup",
            mode: QueryMode::Point {
                table: table.to_string(),
                pk_col: pk_col.to_string(),
                keys: Vec::new(),
                cursor: AtomicUsize::new(0),
            },
            db: None,
            meta: None,
        }
    }

    /// Precompute the point-lookup key sequence from the dataset's row count,
    /// drawn in distribution+seed order so runs are reproducible (SPEC §6).
    fn build_point_keys(rows: u64, distribution: &str, seed: u64) -> anyhow::Result<Vec<String>> {
        // Sample a bounded pool of keys (not all rows) — enough to exercise the
        // distribution without holding millions of strings.
        let pool = rows.min(100_000).max(1);
        let mut gen = KeyGen::new(distribution, pool, seed)?;
        let count = 10_000usize;
        Ok((0..count)
            .map(|_| {
                let idx = gen.next_key();
                format!("k{idx:016x}")
            })
            .collect())
    }
}

#[async_trait]
impl Workload for ReadWorkload {
    fn name(&self) -> &'static str {
        self.name
    }

    fn dataset_meta(&self) -> Option<DatasetMeta> {
        self.meta
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
        // Capture on-disk size + corpus rows so the runner can report them
        // (the codec sweep's size-vs-speed table, SPEC §10).
        self.meta = Some(DatasetMeta {
            bytes: manifest.bytes,
            rows: manifest.rows,
        });

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

        // Point lookups need their key sequence built from the dataset size.
        if let QueryMode::Point { keys, .. } = &mut self.mode {
            *keys = Self::build_point_keys(manifest.rows, &ctx.distribution, ctx.seed)?;
        }
        Ok(())
    }

    async fn op(&self, _worker: usize) -> anyhow::Result<OpRows> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("{}: setup() not called", self.name))?;

        // Form this op's query. Scans use the fixed query; point lookups pick the
        // next key from the precomputed sequence.
        let query = match &self.mode {
            QueryMode::Fixed(q) => q.clone(),
            QueryMode::Point {
                table,
                pk_col,
                keys,
                cursor,
            } => {
                let i = cursor.fetch_add(1, Ordering::Relaxed) % keys.len().max(1);
                let key = keys.get(i).map(String::as_str).unwrap_or("k0");
                format!("SELECT * FROM {table} WHERE {pk_col} = '{key}'")
            }
        };

        // Stream rows through a bounded channel so a full scan stays memory-
        // bounded (SPEC §7 <128 MB target for streaming reads).
        let mut iter = db
            .execute_streaming(&query, StreamingConfig::default())
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
