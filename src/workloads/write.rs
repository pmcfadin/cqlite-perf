//! Write suite (SPEC §6). The write suite doubles as the volume generator
//! (SPEC §8.2): benchmarking the write path and producing an SSTable corpus
//! are the same run.
//!
//! - `write.ingest` — sustained `we.write(mutation)`, WAL-on (durable) and
//!   WAL-off (`write.ingest_waloff`, fsync skipped) variants. WAL-on isolates
//!   the per-write fsync cost; WAL-off the CPU-bound memtable path.
//! - `write.flush` — fill the memtable to threshold, time `we.flush()`; report
//!   flush MB/sec and per-flush latency.
//! - `write.compaction` — build K SSTables, then drive `we.maintenance_step()`
//!   under an STCS policy until convergence; report compaction wall-time,
//!   read-amp (before/after), and output size.
//!
//! `write.ingest` keeps the single-engine-behind-a-`Mutex` shape here; M2
//! Issue #2 gives each worker its own engine for true write concurrency.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cqlite_core::schema::{Column, KeyColumn, TableSchema};
use cqlite_core::storage::write_engine::merge_policy::STCSPolicy;
use cqlite_core::storage::write_engine::mutation::{CellOperation, Mutation, PartitionKey, TableId};
use cqlite_core::storage::write_engine::{Durability, WriteEngine, WriteEngineConfig};
use cqlite_core::types::Value;
use tokio::sync::Mutex;

use super::{OpRows, RunContext, Workload};

const KEYSPACE: &str = "perf";
const TABLE: &str = "ingest";
pub(crate) const FLUSH_THRESHOLD: usize = 8 * 1024 * 1024; // 8 MB memtable
pub(crate) const HARD_LIMIT: usize = 64 * 1024 * 1024; // 64 MB before writes are rejected
const PAYLOAD_BYTES: usize = 256; // ~256 B value column, so rows have realistic heft

/// A small basic-types schema: a text partition key plus two value columns.
/// Mirrors the shape of `schemas/basic.cql`.
pub(crate) fn basic_schema() -> TableSchema {
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

/// Build one mutation for sequence number `n` (~256 B payload).
pub(crate) fn make_mutation(n: u64) -> Mutation {
    let id = format!("k{n:016x}");
    let name = format!("name-{n}");
    let payload = "x".repeat(PAYLOAD_BYTES);
    Mutation::new(
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
    )
}

/// Count `*-Data.db` SSTable component files under `dir` (recursively) and sum
/// their sizes. The Data.db count is the read amplification — how many SSTables
/// a point read may have to touch.
fn count_sstables(dir: &Path) -> (u64, u64) {
    let mut count = 0u64;
    let mut bytes = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("-Data.db"))
            {
                count += 1;
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    (count, bytes)
}

// ---------------------------------------------------------------------------
// write.ingest (WAL-on / WAL-off)
// ---------------------------------------------------------------------------

/// Sustained `WriteEngine::write` ingest (SPEC §6, write.ingest → writes/sec).
///
/// `WriteEngine` is `&mut self` per write and explicitly single-writer, so each
/// worker gets **its own engine** in its own data/WAL subdir (M2 Issue #2).
/// Worker `w` only ever touches `engines[w]`, so the per-engine `Mutex` (needed
/// because `op` is `&self`) is always uncontended — concurrency numbers reflect
/// N independent engines writing in parallel, not lock contention. At WAL-on,
/// the scaling ceiling becomes shared-disk fsync throughput, which is the real
/// thing to measure.
pub struct WriteIngest {
    name: &'static str,
    durability: Durability,
    /// One engine per worker; index by the worker id passed to `op`.
    engines: Vec<Mutex<WriteEngine>>,
    /// One write counter per worker (no cross-worker sharing → no contention).
    /// Keys are unique within each engine; collisions across engines are fine,
    /// they are separate corpora.
    counters: Vec<AtomicU64>,
    /// Held to keep the scratch directory (all worker subdirs) alive for the run.
    _scratch: Option<tempfile::TempDir>,
}

impl WriteIngest {
    /// WAL-on: every `write` appends + fsyncs the WAL (durable, fsync-bound).
    pub fn wal_on() -> Self {
        Self::with("write.ingest", Durability::SyncEachWrite)
    }

    /// WAL-off: skip WAL append + fsync (memtable-only, CPU-bound). Data is
    /// durable only after a `flush` — a benchmarking-only mode (SPEC §6).
    pub fn wal_off() -> Self {
        Self::with("write.ingest_waloff", Durability::Disabled)
    }

    fn with(name: &'static str, durability: Durability) -> Self {
        Self {
            name,
            durability,
            engines: Vec::new(),
            counters: Vec::new(),
            _scratch: None,
        }
    }
}

#[async_trait]
impl Workload for WriteIngest {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn setup(&mut self, ctx: &RunContext) -> anyhow::Result<()> {
        let scratch = tempfile::Builder::new()
            .prefix("cqlite-perf-write-ingest-")
            .tempdir_in(&ctx.work_dir)?;

        // One independent engine per worker, each in its own data/wal subdir.
        let workers = ctx.concurrency.max(1);
        let mut engines = Vec::with_capacity(workers);
        let mut counters = Vec::with_capacity(workers);
        for w in 0..workers {
            let data_dir = scratch.path().join(format!("w{w}/data"));
            let wal_dir = scratch.path().join(format!("w{w}/wal"));
            std::fs::create_dir_all(&data_dir)?;
            std::fs::create_dir_all(&wal_dir)?;

            let config = WriteEngineConfig::new(data_dir, wal_dir, basic_schema())
                .with_flush_threshold(FLUSH_THRESHOLD)
                .with_hard_limit(HARD_LIMIT)
                .with_durability(self.durability);
            let engine =
                WriteEngine::new(config).map_err(|e| anyhow::anyhow!("WriteEngine::new: {e}"))?;
            engines.push(Mutex::new(engine));
            counters.push(AtomicU64::new(0));
        }

        self.engines = engines;
        self.counters = counters;
        self._scratch = Some(scratch);
        Ok(())
    }

    async fn op(&self, worker: usize) -> anyhow::Result<OpRows> {
        let engine = self
            .engines
            .get(worker)
            .ok_or_else(|| anyhow::anyhow!("{}: no engine for worker {worker}", self.name))?;
        // Per-worker counter; only this worker reads/writes it.
        let n = self.counters[worker].fetch_add(1, Ordering::Relaxed);
        let mutation = make_mutation(n);

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
        for engine in self.engines.drain(..) {
            let mut guard = engine.lock().await;
            let _ = guard.flush().await;
            guard
                .close()
                .await
                .map_err(|e| anyhow::anyhow!("WriteEngine::close: {e}"))?;
        }
        self.counters.clear();
        self._scratch = None;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// write.flush
// ---------------------------------------------------------------------------

/// `write.flush` (SPEC §6): each `op()` fills the memtable to the flush
/// threshold, then times a single `we.flush()`. Reports flush MB/sec (logical
/// memtable volume / flush wall-time) and per-flush latency.
///
/// The fill uses WAL-off so the measurement isolates the flush (SSTable write)
/// cost from per-write fsync — otherwise filling 8 MB at the ~280 writes/sec
/// WAL-on rate would dominate and starve the metric. Run with `--warmup 0` so
/// every flush counts toward the reported numbers.
pub struct WriteFlush {
    engine: Option<Mutex<WriteEngine>>,
    counter: AtomicU64,
    flush_count: AtomicU64,
    /// Logical bytes flushed (pre-flush memtable size), summed.
    flush_logical_bytes: AtomicU64,
    /// On-disk Data.db bytes produced, summed (shows compression effect).
    flush_ondisk_bytes: AtomicU64,
    /// Total wall-time spent inside `flush()`, summed (nanos).
    flush_nanos: AtomicU64,
    _scratch: Option<tempfile::TempDir>,
}

impl WriteFlush {
    pub fn new() -> Self {
        Self {
            engine: None,
            counter: AtomicU64::new(0),
            flush_count: AtomicU64::new(0),
            flush_logical_bytes: AtomicU64::new(0),
            flush_ondisk_bytes: AtomicU64::new(0),
            flush_nanos: AtomicU64::new(0),
            _scratch: None,
        }
    }
}

#[async_trait]
impl Workload for WriteFlush {
    fn name(&self) -> &'static str {
        "write.flush"
    }

    async fn setup(&mut self, ctx: &RunContext) -> anyhow::Result<()> {
        let scratch = tempfile::Builder::new()
            .prefix("cqlite-perf-write-flush-")
            .tempdir_in(&ctx.work_dir)?;
        let data_dir = scratch.path().join("data");
        let wal_dir = scratch.path().join("wal");
        std::fs::create_dir_all(&data_dir)?;
        std::fs::create_dir_all(&wal_dir)?;

        let config = WriteEngineConfig::new(data_dir, wal_dir, basic_schema())
            .with_flush_threshold(FLUSH_THRESHOLD)
            // Hard limit comfortably above threshold; we flush explicitly.
            .with_hard_limit(HARD_LIMIT)
            .with_durability(Durability::Disabled);

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
            .ok_or_else(|| anyhow::anyhow!("write.flush: setup() not called"))?;
        let mut guard = engine.lock().await;

        // Fill the memtable to the flush threshold (WAL-off, so this is fast).
        let mut rows: u64 = 0;
        while guard.memtable_size() < FLUSH_THRESHOLD {
            let n = self.counter.fetch_add(1, Ordering::Relaxed);
            guard
                .write(make_mutation(n))
                .map_err(|e| anyhow::anyhow!("WriteEngine::write: {e}"))?;
            rows += 1;
        }

        // Time the flush itself.
        let logical = guard.memtable_size() as u64;
        let t0 = Instant::now();
        let info = guard
            .flush()
            .await
            .map_err(|e| anyhow::anyhow!("WriteEngine::flush: {e}"))?;
        let dt = t0.elapsed();

        self.flush_count.fetch_add(1, Ordering::Relaxed);
        self.flush_logical_bytes.fetch_add(logical, Ordering::Relaxed);
        self.flush_ondisk_bytes
            .fetch_add(info.map(|i| i.data_size).unwrap_or(0), Ordering::Relaxed);
        self.flush_nanos
            .fetch_add(dt.as_nanos() as u64, Ordering::Relaxed);

        Ok(rows)
    }

    fn custom_metrics(&self) -> BTreeMap<String, f64> {
        let count = self.flush_count.load(Ordering::Relaxed);
        let mut m = BTreeMap::new();
        if count == 0 {
            return m;
        }
        let logical = self.flush_logical_bytes.load(Ordering::Relaxed) as f64;
        let ondisk = self.flush_ondisk_bytes.load(Ordering::Relaxed) as f64;
        let secs = self.flush_nanos.load(Ordering::Relaxed) as f64 / 1e9;
        const MB: f64 = 1024.0 * 1024.0;
        if secs > 0.0 {
            m.insert("flush.mb_per_sec".to_string(), logical / MB / secs);
        }
        m.insert(
            "flush.latency_ms_mean".to_string(),
            secs * 1000.0 / count as f64,
        );
        m.insert("flush.ondisk_mb_mean".to_string(), ondisk / MB / count as f64);
        m.insert("flush.count".to_string(), count as f64);
        m
    }

    async fn teardown(&mut self) -> anyhow::Result<()> {
        if let Some(engine) = self.engine.take() {
            let mut guard = engine.lock().await;
            guard
                .close()
                .await
                .map_err(|e| anyhow::anyhow!("WriteEngine::close: {e}"))?;
        }
        self._scratch = None;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// write.compaction
// ---------------------------------------------------------------------------

const COMPACT_FLUSH_THRESHOLD: usize = 4 * 1024 * 1024; // 4 MB per SSTable
const COMPACT_K_SSTABLES: u64 = 4; // SSTables built per compaction cycle

/// `write.compaction` (SPEC §6): each `op()` builds a fresh set of K SSTables,
/// then drives `we.maintenance_step()` under an STCS policy until the merge
/// converges. Reports compaction wall-time, read-amp before/after, output size,
/// and merge throughput.
///
/// Each op is an independent compaction cycle in its own temp dir, so the
/// metric is repeatable across the trial loop. Concurrency is pinned to 1
/// (compaction is a single-engine operation). Run with `--warmup 0`.
pub struct WriteCompaction {
    work_dir: std::path::PathBuf,
    /// Compaction cycles measured.
    cycles: AtomicU64,
    wall_nanos: AtomicU64,
    read_amp_before: AtomicU64,
    read_amp_after: AtomicU64,
    output_bytes: AtomicU64,
    rows_merged: AtomicU64,
}

impl WriteCompaction {
    pub fn new() -> Self {
        Self {
            work_dir: std::path::PathBuf::new(),
            cycles: AtomicU64::new(0),
            wall_nanos: AtomicU64::new(0),
            read_amp_before: AtomicU64::new(0),
            read_amp_after: AtomicU64::new(0),
            output_bytes: AtomicU64::new(0),
            rows_merged: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl Workload for WriteCompaction {
    fn name(&self) -> &'static str {
        "write.compaction"
    }

    async fn setup(&mut self, ctx: &RunContext) -> anyhow::Result<()> {
        self.work_dir = ctx.work_dir.clone();
        Ok(())
    }

    async fn op(&self, _worker: usize) -> anyhow::Result<OpRows> {
        // Fresh engine + temp dir per cycle, so each measurement is independent.
        let scratch = tempfile::Builder::new()
            .prefix("cqlite-perf-write-compaction-")
            .tempdir_in(&self.work_dir)?;
        let data_dir = scratch.path().join("data");
        let wal_dir = scratch.path().join("wal");
        std::fs::create_dir_all(&data_dir)?;
        std::fs::create_dir_all(&wal_dir)?;

        let config = WriteEngineConfig::new(data_dir.clone(), wal_dir, basic_schema())
            .with_flush_threshold(COMPACT_FLUSH_THRESHOLD)
            .with_hard_limit(HARD_LIMIT)
            .with_durability(Durability::Disabled);
        let mut engine =
            WriteEngine::new(config).map_err(|e| anyhow::anyhow!("WriteEngine::new: {e}"))?;

        // Build K SSTables: fill the memtable to threshold and flush, K times.
        let mut n: u64 = 0;
        for _ in 0..COMPACT_K_SSTABLES {
            while engine.memtable_size() < COMPACT_FLUSH_THRESHOLD {
                engine
                    .write(make_mutation(n))
                    .map_err(|e| anyhow::anyhow!("WriteEngine::write: {e}"))?;
                n += 1;
            }
            engine
                .flush()
                .await
                .map_err(|e| anyhow::anyhow!("WriteEngine::flush: {e}"))?;
        }

        let (ra_before, _) = count_sstables(&data_dir);

        // STCS with min_threshold=2 so a handful of equally-small SSTables is
        // enough to trigger (and fully merge) a compaction.
        let policy = STCSPolicy::new(2, 32, 0.5, 1.5, STCSPolicy::DEFAULT_MIN_SSTABLE_SIZE)
            .map_err(|e| anyhow::anyhow!("STCSPolicy::new: {e}"))?;
        engine
            .set_merge_policy(Box::new(policy))
            .map_err(|e| anyhow::anyhow!("set_merge_policy: {e}"))?;

        // Drive maintenance until convergence, timing the loop. Bounded by a
        // wall-clock safety cap so a misbehaving policy can't spin forever.
        //
        // `maintenance_step` is sync but internally bridges to async via
        // `handle.block_on`, which PANICS when called from a tokio worker thread
        // (cqlite #587). We sidestep it by hopping onto a `spawn_blocking` thread
        // (no runtime entered), so the engine takes its own-runtime branch. A
        // generous per-step budget keeps the number of that-branch runtime
        // spin-ups (and their overhead in the measured wall-time) low. Repro:
        // `cargo run --example probe_compaction`.
        let (mut engine, merged, wall) = tokio::task::spawn_blocking(move || {
            let mut engine = engine;
            let budget = Duration::from_secs(5);
            let cap = Duration::from_secs(60);
            let t0 = Instant::now();
            let mut merged: u64 = 0;
            let res = loop {
                match engine.maintenance_step(budget) {
                    Ok(report) => {
                        merged += report.rows_merged;
                        if !report.pending_compaction {
                            break Ok(());
                        }
                        if t0.elapsed() > cap {
                            break Err(anyhow::anyhow!("compaction did not converge in {cap:?}"));
                        }
                    }
                    Err(e) => break Err(anyhow::anyhow!("maintenance_step: {e}")),
                }
            };
            let wall = t0.elapsed();
            res.map(|()| (engine, merged, wall))
        })
        .await
        .map_err(|e| anyhow::anyhow!("compaction worker join: {e}"))??;

        let (ra_after, out_bytes) = count_sstables(&data_dir);

        self.cycles.fetch_add(1, Ordering::Relaxed);
        self.wall_nanos
            .fetch_add(wall.as_nanos() as u64, Ordering::Relaxed);
        self.read_amp_before.fetch_add(ra_before, Ordering::Relaxed);
        self.read_amp_after.fetch_add(ra_after, Ordering::Relaxed);
        self.output_bytes.fetch_add(out_bytes, Ordering::Relaxed);
        self.rows_merged.fetch_add(merged, Ordering::Relaxed);

        engine
            .close()
            .await
            .map_err(|e| anyhow::anyhow!("WriteEngine::close: {e}"))?;
        // scratch drops here, removing the temp dir.
        Ok(merged)
    }

    fn custom_metrics(&self) -> BTreeMap<String, f64> {
        let cycles = self.cycles.load(Ordering::Relaxed);
        let mut m = BTreeMap::new();
        if cycles == 0 {
            return m;
        }
        let c = cycles as f64;
        let wall_secs = self.wall_nanos.load(Ordering::Relaxed) as f64 / 1e9;
        let out_bytes = self.output_bytes.load(Ordering::Relaxed) as f64;
        const MB: f64 = 1024.0 * 1024.0;
        m.insert("compaction.wall_ms_mean".to_string(), wall_secs * 1000.0 / c);
        m.insert(
            "compaction.read_amp_before".to_string(),
            self.read_amp_before.load(Ordering::Relaxed) as f64 / c,
        );
        m.insert(
            "compaction.read_amp_after".to_string(),
            self.read_amp_after.load(Ordering::Relaxed) as f64 / c,
        );
        m.insert("compaction.output_mb_mean".to_string(), out_bytes / MB / c);
        if wall_secs > 0.0 {
            m.insert(
                "compaction.merge_mb_per_sec".to_string(),
                out_bytes / MB / wall_secs,
            );
        }
        m.insert("compaction.cycles".to_string(), c);
        m
    }

    async fn teardown(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}
