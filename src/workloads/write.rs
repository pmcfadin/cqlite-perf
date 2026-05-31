//! Write suite (SPEC §6). The write suite doubles as the volume generator
//! (SPEC §8.2): benchmarking the write path and producing an SSTable corpus
//! are the same run.
//!
//! M0 ships `write.ingest` — the workload that proves the runner loop
//! end-to-end with zero external setup (no Docker, no pre-built corpus). The
//! flush/compaction workloads and per-worker engines arrive in M2.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use cqlite_core::schema::{Column, KeyColumn, TableSchema};
use cqlite_core::storage::write_engine::mutation::{CellOperation, Mutation, PartitionKey, TableId};
use cqlite_core::storage::write_engine::{WriteEngine, WriteEngineConfig};
use cqlite_core::types::Value;
use tokio::sync::Mutex;

use super::{OpRows, RunContext, Workload};

const KEYSPACE: &str = "perf";
const TABLE: &str = "ingest";
const FLUSH_THRESHOLD: usize = 8 * 1024 * 1024; // 8 MB memtable
const HARD_LIMIT: usize = 64 * 1024 * 1024; // 64 MB before writes are rejected

/// Sustained `WriteEngine::write` ingest (SPEC §6, write.ingest → writes/sec).
///
/// The engine requires `&mut self` per write, so for M0 it lives behind a
/// `Mutex`. That serializes writers — acceptable for the low-concurrency M0
/// proof; M2 replaces this with per-worker engines for true write concurrency.
pub struct WriteIngest {
    engine: Option<Mutex<WriteEngine>>,
    counter: AtomicU64,
    /// Held to keep the scratch directory alive for the run.
    _scratch: Option<tempfile::TempDir>,
}

impl WriteIngest {
    pub fn new() -> Self {
        Self {
            engine: None,
            counter: AtomicU64::new(0),
            _scratch: None,
        }
    }
}

impl Default for WriteIngest {
    fn default() -> Self {
        Self::new()
    }
}

/// A small basic-types schema: a text partition key plus two value columns.
/// Mirrors the shape of `schemas/basic.cql`.
fn basic_schema() -> TableSchema {
    let col = |name: &str| Column {
        name: name.to_string(),
        data_type: "text".to_string(),
        nullable: false,
        default: None,
        is_static: false,
    };
    TableSchema {
        keyspace: KEYSPACE.to_string(),
        table: TABLE.to_string(),
        partition_keys: vec![KeyColumn {
            name: "id".to_string(),
            data_type: "text".to_string(),
            position: 0,
        }],
        clustering_keys: vec![],
        columns: vec![col("id"), col("name"), col("payload")],
        comments: HashMap::new(),
    }
}

#[async_trait]
impl Workload for WriteIngest {
    fn name(&self) -> &'static str {
        "write.ingest"
    }

    async fn setup(&mut self, ctx: &RunContext) -> anyhow::Result<()> {
        let scratch = tempfile::Builder::new()
            .prefix("cqlite-perf-write-ingest-")
            .tempdir_in(&ctx.work_dir)?;
        let data_dir = scratch.path().join("data");
        let wal_dir = scratch.path().join("wal");
        std::fs::create_dir_all(&data_dir)?;
        std::fs::create_dir_all(&wal_dir)?;

        let config = WriteEngineConfig::new(data_dir, wal_dir, basic_schema())
            .with_flush_threshold(FLUSH_THRESHOLD)
            .with_hard_limit(HARD_LIMIT);

        let engine =
            WriteEngine::new(config).map_err(|e| anyhow::anyhow!("WriteEngine::new: {e}"))?;

        self.engine = Some(Mutex::new(engine));
        self._scratch = Some(scratch);
        Ok(())
    }

    async fn op(&self, _worker: usize) -> anyhow::Result<OpRows> {
        let engine = self
            .engine
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("write.ingest: setup() not called"))?;

        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let id = format!("k{n:016x}");
        let name = format!("name-{n}");
        // ~256 B payload so rows have realistic heft.
        let payload = "x".repeat(256);

        let mutation = Mutation::new(
            TableId::new(KEYSPACE, TABLE),
            PartitionKey::new(vec![("id".to_string(), Value::Text(id))]),
            None,
            vec![
                CellOperation::Write {
                    column: "name".to_string(),
                    value: Value::Text(name),
                },
                CellOperation::Write {
                    column: "payload".to_string(),
                    value: Value::Text(payload),
                },
            ],
            (n as i64) + 1, // monotonic timestamp_micros
            None,
        );

        let mut guard = engine.lock().await;
        guard
            .write(mutation)
            .map_err(|e| anyhow::anyhow!("WriteEngine::write: {e}"))?;

        // Flush proactively before the memtable reaches the hard limit, which
        // would otherwise start rejecting writes.
        if guard.memtable_size() >= FLUSH_THRESHOLD {
            guard
                .flush()
                .await
                .map_err(|e| anyhow::anyhow!("WriteEngine::flush: {e}"))?;
        }

        Ok(1)
    }

    async fn teardown(&mut self) -> anyhow::Result<()> {
        if let Some(engine) = self.engine.take() {
            let mut guard = engine.lock().await;
            // Best-effort final flush, then close; scratch dir drops after.
            let _ = guard.flush().await;
            guard
                .close()
                .await
                .map_err(|e| anyhow::anyhow!("WriteEngine::close: {e}"))?;
        }
        self._scratch = None;
        Ok(())
    }
}
