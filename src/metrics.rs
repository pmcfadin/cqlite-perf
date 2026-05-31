//! Metrics: HDR latency histograms, the metric envelope (SPEC §7), and
//! aggregation helpers. Every `RunResult` serializes to the same JSON shape so
//! reports, regression diffs, and cross-version comparison stay uniform.

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};

/// Per-worker latency recorder. Records operation latencies in microseconds.
///
/// HDR histograms are merged across workers at the end of a measurement phase
/// to preserve tails (SPEC §5, §9).
pub struct LatencyRecorder {
    hist: Histogram<u64>,
}

impl LatencyRecorder {
    /// Tracks 1µs .. ~1 hour at 3 significant figures. The bounds are valid by
    /// construction, so `new` cannot fail in practice.
    pub fn new() -> Self {
        let hist = Histogram::<u64>::new_with_bounds(1, 3_600_000_000, 3)
            .expect("valid HDR histogram bounds");
        Self { hist }
    }

    /// Record one operation latency in microseconds. Values above the ceiling
    /// are saturated to the max rather than dropped, so a stalled op still
    /// shows up in the tail.
    pub fn record_micros(&mut self, micros: u64) {
        let v = micros.clamp(1, 3_600_000_000);
        self.hist.saturating_record(v);
    }

    pub fn merge(&mut self, other: &LatencyRecorder) {
        self.hist.add(&other.hist).expect("compatible histograms");
    }

    pub fn count(&self) -> u64 {
        self.hist.len()
    }

    pub fn snapshot(&self) -> LatencyUs {
        LatencyUs {
            p50: self.hist.value_at_quantile(0.50),
            p90: self.hist.value_at_quantile(0.90),
            p95: self.hist.value_at_quantile(0.95),
            p99: self.hist.value_at_quantile(0.99),
            p999: self.hist.value_at_quantile(0.999),
            max: self.hist.max(),
        }
    }
}

impl Default for LatencyRecorder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Metric envelope (SPEC §7)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub workload: String,
    pub cqlite_version: String,
    pub cqlite_git_sha: Option<String>,
    pub harness_version: String,
    pub dataset: DatasetInfo,
    pub distribution: String,
    pub concurrency: usize,
    /// "warm" or "cold"
    pub cache: String,
    pub throughput: Throughput,
    pub latency_us: LatencyUs,
    pub resource: Resource,
    pub trials: u32,
    pub variance: Variance,
    pub host: HostInfo,
    pub duration_secs: u64,
    pub warmup_secs: u64,
    pub seed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetInfo {
    pub tier: String,
    pub rows: u64,
    pub bytes: u64,
    pub codec: String,
    pub schema: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Throughput {
    pub ops_per_sec: f64,
    pub rows_per_sec: f64,
    pub mb_per_sec: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyUs {
    pub p50: u64,
    pub p90: u64,
    pub p95: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub peak_rss_bytes: u64,
    pub cpu_pct_mean: f64,
    /// Populated only in Phase 3 profiling runs (via dhat).
    pub alloc_bytes: Option<u64>,
    pub alloc_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variance {
    /// Coefficient of variation of ops/sec across trials.
    pub ops_per_sec_cv: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInfo {
    pub cpu: String,
    pub cores: usize,
    pub ram_gb: u64,
    pub os: String,
}

impl HostInfo {
    /// Best-effort host description. Fields that can't be determined fall back
    /// to coarse defaults rather than failing the run.
    pub fn detect() -> Self {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0);
        HostInfo {
            cpu: std::env::consts::ARCH.to_string(),
            cores,
            ram_gb: 0,
            os: format!("{} {}", std::env::consts::OS, os_release()),
        }
    }
}

fn os_release() -> String {
    std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Coefficient of variation (stddev / mean) for a set of trial throughputs.
/// Returns 0.0 for fewer than two samples or a zero mean.
pub fn coefficient_of_variation(samples: &[f64]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    if mean == 0.0 {
        return 0.0;
    }
    let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / samples.len() as f64;
    var.sqrt() / mean
}

/// Median of a set of samples (the headline trial value).
pub fn median(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = s.len() / 2;
    if s.len() % 2 == 0 {
        (s[mid - 1] + s[mid]) / 2.0
    } else {
        s[mid]
    }
}
