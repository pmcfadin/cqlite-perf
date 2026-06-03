//! Mixed suite (SPEC §6): `read_while_write` and `open_loop`. Both run reader
//! workers (streaming `SELECT *` scans over an existing corpus) alongside writer
//! workers (ingest into separate per-writer engines), and report **read latency
//! under write load**.
//!
//! The driving strategy is the difference, and it lives in the runner's cohort
//! plan ([`Workload::work_plan`]), not here:
//!
//! - `mixed.read_while_write` — readers and writers both closed-loop. Headline:
//!   `read.p99_us` while writers hammer.
//! - `mixed.open_loop` — writers driven **open-loop** at a fixed target rate with
//!   coordinated-omission correction (so a flush stall shows up as writer tail
//!   latency, not a hidden slowdown); readers closed-loop. Headline: does
//!   `read.p99_us` hold, and does the writer cohort keep up (`write.ops_per_sec`
//!   vs `write.target_ops_per_sec`).
//!
//! Readers use a full scan (`read.full_scan`), which works in v0.10.0 — NOT
//! `point_lookup`, which is blocked by the TEXT-PK bug (cqlite #586 / harness
//! #13). Writers use WAL-off engines so the write load is CPU/flush pressure
//! that competes with scan decode. Run mixed workloads with `--warmup 0` (the
//! open-loop schedule starts at the measurement, and there is no per-op state to
//! prime).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use cqlite_core::ingestion::{ingest, IngestionConfig};
use cqlite_core::query::result::StreamingConfig;
use cqlite_core::storage::write_engine::{Durability, WriteEngine, WriteEngineConfig};
use cqlite_core::{Config, Database};
use tokio::sync::Mutex;

use super::write::{basic_schema, make_mutation, FLUSH_THRESHOLD, HARD_LIMIT};
use super::{Cohort, OpRows, RunContext, WorkPlan, Workload};

/// Writer workers in a mixed run (the rest of `concurrency` are readers).
const WRITERS: usize = 2;
/// Total target write rate for `mixed.open_loop` (writes/sec across writers).
/// WAL-off writers can far exceed this single-threaded, so the interesting
/// question is whether the *reads* hold up at this sustained write rate.
const OPEN_LOOP_TARGET_RATE: f64 = 100_000.0;
/// Read corpus the scanners run over (must exist locally; the read suite uses it).
const READ_TABLE: &str = "basic";
const READ_SCHEMA: &str = "basic";
const MANIFESTS_DIR: &str = "datasets/manifests";

#[derive(Clone, Copy)]
enum Mode {
    ReadWhileWrite,
    OpenLoop,
}

/// Writer count for a given concurrency: `WRITERS`, but always leaving at least
/// one reader and at least one writer.
fn writer_count(concurrency: usize) -> usize {
    WRITERS.min(concurrency.saturating_sub(1)).max(1)
}

pub struct Mixed {
    name: &'static str,
    mode: Mode,
    /// Writer count, fixed at setup from `ctx.concurrency`.
    n_writers: usize,
    read_db: Option<Arc<Database>>,
    writers: Vec<Mutex<WriteEngine>>,
    write_counters: Vec<AtomicU64>,
    /// Total rows scanned by readers (informational).
    read_rows: AtomicU64,
    _scratch: Option<tempfile::TempDir>,
}

impl Mixed {
    pub fn read_while_write() -> Self {
        Self::with("mixed.read_while_write", Mode::ReadWhileWrite)
    }

    pub fn open_loop() -> Self {
        Self::with("mixed.open_loop", Mode::OpenLoop)
    }

    fn with(name: &'static str, mode: Mode) -> Self {
        Self {
            name,
            mode,
            n_writers: 0,
            read_db: None,
            writers: Vec::new(),
            write_counters: Vec::new(),
            read_rows: AtomicU64::new(0),
            _scratch: None,
        }
    }
}

#[async_trait]
impl Workload for Mixed {
    fn name(&self) -> &'static str {
        self.name
    }

    fn work_plan(&self, concurrency: usize) -> Option<WorkPlan> {
        let w = writer_count(concurrency);
        let r = concurrency.saturating_sub(w);
        // Writer cohort first → its workers own ids [0, w); readers own [w, w+r).
        let write_rate = match self.mode {
            Mode::ReadWhileWrite => None,
            Mode::OpenLoop => Some(OPEN_LOOP_TARGET_RATE),
        };
        let mut cohorts = vec![Cohort {
            label: "write",
            workers: w,
            open_loop_rate: write_rate,
        }];
        if r > 0 {
            cohorts.push(Cohort {
                label: "read",
                workers: r,
                open_loop_rate: None,
            });
        }
        Some(WorkPlan { cohorts })
    }

    async fn setup(&mut self, ctx: &RunContext) -> anyhow::Result<()> {
        self.n_writers = writer_count(ctx.concurrency);

        // --- Writer side: one WAL-off engine per writer, separate dirs. ---
        let scratch = tempfile::Builder::new()
            .prefix("cqlite-perf-mixed-")
            .tempdir_in(&ctx.work_dir)?;
        let mut writers = Vec::with_capacity(self.n_writers);
        let mut counters = Vec::with_capacity(self.n_writers);
        for w in 0..self.n_writers {
            let data_dir = scratch.path().join(format!("w{w}/data"));
            let wal_dir = scratch.path().join(format!("w{w}/wal"));
            std::fs::create_dir_all(&data_dir)?;
            std::fs::create_dir_all(&wal_dir)?;
            let config = WriteEngineConfig::new(data_dir, wal_dir, basic_schema())
                .with_flush_threshold(FLUSH_THRESHOLD)
                .with_hard_limit(HARD_LIMIT)
                .with_durability(Durability::Disabled);
            let engine =
                WriteEngine::new(config).map_err(|e| anyhow::anyhow!("WriteEngine::new: {e}"))?;
            writers.push(Mutex::new(engine));
            counters.push(AtomicU64::new(0));
        }
        self.writers = writers;
        self.write_counters = counters;
        self._scratch = Some(scratch);

        // --- Reader side: open the existing basic corpus for full scans. ---
        let repo_root = std::env::current_dir()?;
        let manifests_dir = repo_root.join(MANIFESTS_DIR);
        let q = crate::datasets::ManifestQuery {
            tier: ctx.tier.clone(),
            schema: READ_SCHEMA.to_string(),
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
            .map_err(|e| anyhow::anyhow!("ingest read corpus {}: {e}", manifest.id))?;
        self.read_db = Some(Arc::new(result.database));
        Ok(())
    }

    async fn op(&self, worker: usize) -> anyhow::Result<OpRows> {
        if worker < self.n_writers {
            // Writer role: ingest one mutation into this writer's own engine.
            let engine = &self.writers[worker];
            let n = self.write_counters[worker].fetch_add(1, Ordering::Relaxed);
            let mut guard = engine.lock().await;
            guard
                .write(make_mutation(n))
                .map_err(|e| anyhow::anyhow!("WriteEngine::write: {e}"))?;
            if guard.memtable_size() >= FLUSH_THRESHOLD {
                guard
                    .flush()
                    .await
                    .map_err(|e| anyhow::anyhow!("WriteEngine::flush: {e}"))?;
            }
            Ok(1)
        } else {
            // Reader role: full streaming scan over the static corpus.
            let db = self
                .read_db
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("{}: setup() not called", self.name))?;
            let mut iter = db
                .execute_streaming(&format!("SELECT * FROM {READ_TABLE}"), StreamingConfig::default())
                .await
                .map_err(|e| anyhow::anyhow!("execute_streaming: {e}"))?;
            let mut rows: u64 = 0;
            while let Some(row) = iter.next_async().await {
                row.map_err(|e| anyhow::anyhow!("row decode: {e}"))?;
                rows += 1;
            }
            self.read_rows.fetch_add(rows, Ordering::Relaxed);
            Ok(rows)
        }
    }

    fn custom_metrics(&self) -> BTreeMap<String, f64> {
        // Per-cohort latency/throughput is emitted by the runner from the cohort
        // histograms. Here we add only what the runner can't know: the open-loop
        // target rate, so achieved-vs-target is readable in one place.
        let mut m = BTreeMap::new();
        if matches!(self.mode, Mode::OpenLoop) {
            m.insert(
                "write.target_ops_per_sec".to_string(),
                OPEN_LOOP_TARGET_RATE,
            );
        }
        m
    }

    async fn teardown(&mut self) -> anyhow::Result<()> {
        for engine in self.writers.drain(..) {
            let mut guard = engine.lock().await;
            let _ = guard.flush().await;
            guard
                .close()
                .await
                .map_err(|e| anyhow::anyhow!("WriteEngine::close: {e}"))?;
        }
        self.write_counters.clear();
        if let Some(db) = self.read_db.take() {
            if let Ok(db) = Arc::try_unwrap(db) {
                db.shutdown()
                    .await
                    .map_err(|e| anyhow::anyhow!("Database::shutdown: {e}"))?;
            }
        }
        self._scratch = None;
        Ok(())
    }
}
