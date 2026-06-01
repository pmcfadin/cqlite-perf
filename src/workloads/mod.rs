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

#[async_trait]
pub trait Workload: Send + Sync {
    /// Stable identifier, e.g. "read.point_lookup".
    fn name(&self) -> &'static str;

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

    /// Teardown: shutdown DB, drop temp dirs.
    async fn teardown(&mut self) -> anyhow::Result<()>;
}

/// Construct a workload by its stable name (e.g. "write.ingest").
pub fn build(name: &str) -> anyhow::Result<Box<dyn Workload>> {
    match name {
        "write.ingest" => Ok(Box::new(write::WriteIngest::new())),
        // The table name is resolved from the schema at setup time; for M1 the
        // generated corpus uses a fixed table per schema (see cassandra_gen).
        "read.full_scan" => Ok(Box::new(read::ReadWorkload::full_scan("basic"))),
        "read.point_lookup" => Ok(Box::new(read::ReadWorkload::point_lookup("basic", "id"))),
        // Slice ranges over the wide_rows composite-PK corpus (pk, ck).
        "read.clustering_slice" => Ok(Box::new(read::ReadWorkload::clustering_slice(
            "wide_rows", "pk", "ck",
        ))),
        "read.type_heavy" => Ok(Box::new(read::ReadWorkload::type_heavy("collections"))),
        "read.wide_partition" => Ok(Box::new(read::ReadWorkload::wide_partition("wide_rows"))),
        other => anyhow::bail!(
            "workload '{other}' is not implemented yet (M1 adds read.full_scan; \
             remaining read/mixed variants land incrementally)"
        ),
    }
}
