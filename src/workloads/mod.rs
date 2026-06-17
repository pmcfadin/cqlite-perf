//! Workload trait and the immutable per-run context (SPEC §5).
//!
//! Every suite implements one trait so the runner, metrics, and reporting stay
//! uniform.

use async_trait::async_trait;

pub mod mixed;
pub mod read;
pub mod write;

/// Immutable per-run config handed to every workload (SPEC §5).
#[derive(Debug, Clone)]
pub struct RunContext {
    pub tier: String,
    pub schema: String,
    pub codec: String,
    pub distribution: String,
    pub concurrency: usize,
    pub warmup_secs: u64,
    pub duration_secs: u64,
    pub trials: u32,
    pub seed: u64,
    pub cold_cache: bool,
    /// Scratch directory root for workloads that generate their own data
    /// (write/volume). Read workloads resolve datasets via manifest instead.
    pub work_dir: std::path::PathBuf,
}

/// Outcome of a single `op()` call: how many rows it touched, for rows/sec
/// accounting (SPEC §5).
pub type OpRows = u64;

/// Static dataset facts for a manifest-backed workload: the corpus's on-disk
/// size and row count, captured at setup. The runner reads this to populate
/// `DatasetInfo` so the codec sweep can report on-disk size vs. scan speed
/// (SPEC §10). `None` for workloads that generate their own data (write/mixed).
#[derive(Debug, Clone, Copy)]
pub struct DatasetMeta {
    pub bytes: u64,
    pub rows: u64,
}

/// One role-group of workers within a mixed workload. Worker ids are assigned
/// to cohorts in plan order: the first cohort owns ids `[0, workers)`, the next
/// `[workers, workers + next.workers)`, and so on — so `op(worker)` can dispatch
/// by id. `open_loop_rate = None` drives the cohort closed-loop (next op as soon
/// as the prior returns); `Some(rate)` drives it open-loop at `rate` ops/sec
/// total across the cohort, measuring latency against the *intended* issue time
/// for coordinated-omission correction (SPEC §5/§6).
#[derive(Debug, Clone)]
pub struct Cohort {
    pub label: &'static str,
    pub workers: usize,
    pub open_loop_rate: Option<f64>,
}

/// How a mixed workload splits its workers across roles/driving strategies.
/// The runner drives each cohort with its strategy and reports per-cohort
/// latency + throughput (emitted as `custom["<label>.p99_us"]` etc.).
#[derive(Debug, Clone)]
pub struct WorkPlan {
    pub cohorts: Vec<Cohort>,
}

impl WorkPlan {
    /// A single closed-loop cohort over all workers — the implicit default for
    /// non-mixed workloads (read/write).
    pub fn single(label: &'static str, workers: usize) -> Self {
        WorkPlan {
            cohorts: vec![Cohort {
                label,
                workers: workers.max(1),
                open_loop_rate: None,
            }],
        }
    }
}

#[async_trait]
pub trait Workload: Send + Sync {
    /// Stable identifier, e.g. "read.point_lookup".
    fn name(&self) -> &'static str;

    /// How to split workers across roles for a mixed workload. `None` → a single
    /// closed-loop cohort of all workers (what read/write workloads use). When
    /// `Some`, the runner drives each cohort per its strategy and emits per-cohort
    /// `custom` metrics. Must reflect the same role→id mapping `op()` assumes.
    fn work_plan(&self, _concurrency: usize) -> Option<WorkPlan> {
        None
    }

    /// One-time setup: open DB / build write engine / preselect keys.
    async fn setup(&mut self, ctx: &RunContext) -> anyhow::Result<()>;

    /// On-disk size + corpus row count for manifest-backed workloads, valid
    /// after `setup`. Defaults to `None`; read workloads override it.
    fn dataset_meta(&self) -> Option<DatasetMeta> {
        None
    }

    /// Execute exactly one operation. Timed by the runner.
    /// Returns rows touched (for rows/sec accounting).
    async fn op(&self, worker: usize) -> anyhow::Result<OpRows>;

    /// Workload-specific metrics that don't fit the standard envelope
    /// (throughput/latency/RSS) — e.g. flush MB/sec, compaction wall-time,
    /// read p99 under write load. Keyed by a dotted name (`flush.mb_per_sec`)
    /// so goals.toml can select `custom.<key>`. Read once per trial after the
    /// measurement phase and aggregated (median) across trials by the runner.
    /// Defaults to empty; write/mixed workloads override it.
    fn custom_metrics(&self) -> std::collections::BTreeMap<String, f64> {
        std::collections::BTreeMap::new()
    }

    /// Teardown: shutdown DB, drop temp dirs.
    async fn teardown(&mut self) -> anyhow::Result<()>;
}

/// The corpus schema a read workload is bound to (its table lives in that
/// schema). Lets the runner resolve the right manifest per workload so the
/// whole read suite runs in one invocation, instead of forcing one global
/// `--schema`. `None` → the caller's `--schema` applies (write/mixed).
pub fn default_schema(name: &str) -> Option<&'static str> {
    match name {
        "read.full_scan" | "read.point_lookup" => Some("basic"),
        "read.type_heavy" => Some("collections"),
        "read.wide_partition" | "read.clustering_slice" => Some("wide_rows"),
        _ => None,
    }
}

/// Construct a workload by its stable name (e.g. "write.ingest").
pub fn build(name: &str) -> anyhow::Result<Box<dyn Workload>> {
    match name {
        "write.ingest" => Ok(Box::new(write::WriteIngest::wal_on())),
        "write.ingest_waloff" => Ok(Box::new(write::WriteIngest::wal_off())),
        "write.flush" => Ok(Box::new(write::WriteFlush::new())),
        "write.compaction" => Ok(Box::new(write::WriteCompaction::new())),
        "mixed.read_while_write" => Ok(Box::new(mixed::Mixed::read_while_write())),
        "mixed.open_loop" => Ok(Box::new(mixed::Mixed::open_loop())),
        // The table name is resolved from the schema at setup time; for M1 the
        // generated corpus uses a fixed table per schema (see cassandra_gen).
        // Table names are keyspace-qualified: cqlite v0.11.0 (VG7, cqlite #680)
        // keys table identity by (keyspace, table), so an unqualified `basic`
        // no longer resolves on the query path — it must be `perf.basic`.
        "read.full_scan" => Ok(Box::new(read::ReadWorkload::full_scan("perf.basic"))),
        "read.point_lookup" => Ok(Box::new(read::ReadWorkload::point_lookup("perf.basic", "id"))),
        // Slice ranges over the wide_rows composite-PK corpus (pk, ck).
        "read.clustering_slice" => Ok(Box::new(read::ReadWorkload::clustering_slice(
            "perf.wide_rows", "pk", "ck",
        ))),
        "read.type_heavy" => Ok(Box::new(read::ReadWorkload::type_heavy("perf.collections"))),
        "read.wide_partition" => Ok(Box::new(read::ReadWorkload::wide_partition("perf.wide_rows"))),
        other => anyhow::bail!(
            "workload '{other}' is not implemented yet (M1 adds read.full_scan; \
             remaining read/mixed variants land incrementally)"
        ),
    }
}
