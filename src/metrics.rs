//! Metrics: HDR latency histograms, the metric envelope (SPEC §7), and
//! aggregation helpers. Every `RunResult` serializes to the same JSON shape so
//! reports, regression diffs, and cross-version comparison stay uniform.

use std::collections::BTreeMap;

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
    /// Workload-specific metrics outside the standard envelope (e.g.
    /// `flush.mb_per_sec`, `compaction.wall_ms`, `read.p99_us`). Empty for
    /// read workloads; populated by write/mixed. Selectable in goals.toml as
    /// `custom.<key>`. Median across trials.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub custom: BTreeMap<String, f64>,
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

// ---------------------------------------------------------------------------
// Interval snapshots (soak time series, issue #31)
// ---------------------------------------------------------------------------

/// One window of a soak time series: per-cohort ops/latency deltas plus a
/// process-RSS sample, emitted as a line of `intervals.jsonl` next to
/// `results.jsonl`. The per-window histogram resets on each snapshot, so
/// `latency_us` is window-local (drift is visible), while the envelope in
/// `RunResult` stays cumulative.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalRecord {
    pub workload: String,
    pub cohort: String,
    /// 1-based trial this window belongs to.
    pub trial: u32,
    /// Seconds from measurement start to the END of this window.
    pub elapsed_s: u64,
    /// Actual window length (the final window of a run may be partial).
    pub window_s: f64,
    pub ops: u64,
    pub rows: u64,
    pub throughput_ops_per_sec: f64,
    pub latency_us: LatencyUs,
    /// Process RSS sampled at the window boundary (process-wide, so identical
    /// across cohort lines of the same window).
    pub rss_bytes: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub custom: BTreeMap<String, f64>,
}

// ---------------------------------------------------------------------------
// Trend derivation (issue #31) — pure math over a window series, quartile-based
// so a single noisy window can't flip a verdict. Results land in
// `RunResult.custom` as `trend.*` and are gated via goals.toml (issue #32).
// ---------------------------------------------------------------------------

/// Mean of the last quartile of values ÷ mean of the first quartile.
/// `1.0` = flat, `< 1.0` = decay. Needs ≥ 4 windows, else `None`.
pub fn quartile_ratio(values: &[f64]) -> Option<f64> {
    let n = values.len();
    if n < 4 {
        return None;
    }
    let q = n / 4;
    let first: f64 = values[..q].iter().sum::<f64>() / q as f64;
    let last: f64 = values[n - q..].iter().sum::<f64>() / q as f64;
    if first == 0.0 {
        return None;
    }
    Some(last / first)
}

/// Least-squares slope of `(secs, value)` samples, converted to per-hour.
/// Needs ≥ 2 distinct x values, else `None`.
pub fn slope_per_hour(samples: &[(f64, f64)]) -> Option<f64> {
    let n = samples.len() as f64;
    if samples.len() < 2 {
        return None;
    }
    let mean_x = samples.iter().map(|(x, _)| x).sum::<f64>() / n;
    let mean_y = samples.iter().map(|(_, y)| y).sum::<f64>() / n;
    let sxx: f64 = samples.iter().map(|(x, _)| (x - mean_x).powi(2)).sum();
    if sxx == 0.0 {
        return None;
    }
    let sxy: f64 = samples
        .iter()
        .map(|(x, y)| (x - mean_x) * (y - mean_y))
        .sum();
    Some(sxy / sxx * 3600.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quartile_ratio_flat_series_is_one() {
        let v = vec![100.0; 12];
        assert_eq!(quartile_ratio(&v), Some(1.0));
    }

    #[test]
    fn quartile_ratio_detects_decay() {
        // 8 windows: first quartile (2) mean 100, last quartile (2) mean 80.
        let v = vec![100.0, 100.0, 95.0, 92.0, 88.0, 85.0, 81.0, 79.0];
        let r = quartile_ratio(&v).unwrap();
        assert!((r - 0.80).abs() < 1e-9, "got {r}");
    }

    #[test]
    fn quartile_ratio_single_spike_does_not_flip_verdict() {
        // Flat except one noisy window inside the body — quartiles ignore it.
        let mut v = vec![100.0; 12];
        v[6] = 10.0;
        assert_eq!(quartile_ratio(&v), Some(1.0));
    }

    #[test]
    fn quartile_ratio_needs_four_windows() {
        assert_eq!(quartile_ratio(&[1.0, 2.0, 3.0]), None);
        assert_eq!(quartile_ratio(&[]), None);
    }

    #[test]
    fn slope_flat_is_zero() {
        let s: Vec<(f64, f64)> = (0..10).map(|i| (i as f64 * 60.0, 500.0)).collect();
        assert!(slope_per_hour(&s).unwrap().abs() < 1e-9);
    }

    #[test]
    fn slope_recovers_known_growth() {
        // +2 MB every 60 s → 120 MB/h.
        let s: Vec<(f64, f64)> = (0..20).map(|i| (i as f64 * 60.0, 100.0 + 2.0 * i as f64)).collect();
        let m = slope_per_hour(&s).unwrap();
        assert!((m - 120.0).abs() < 1e-6, "got {m}");
    }

    #[test]
    fn slope_needs_two_distinct_points() {
        assert_eq!(slope_per_hour(&[(0.0, 1.0)]), None);
        assert_eq!(slope_per_hour(&[(5.0, 1.0), (5.0, 2.0)]), None);
    }
}
