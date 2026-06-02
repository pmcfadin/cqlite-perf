//! The measurement loop (SPEC §5): warmup, duration-based measurement, trials,
//! closed-loop workers, and best-effort resource sampling.
//!
//! Open-loop (target-rate) driving with coordinated-omission correction is a
//! mixed-suite concern and lands in M2.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sysinfo::{Pid, System};
use tokio::sync::Mutex;

use crate::metrics::{
    coefficient_of_variation, median, DatasetInfo, HostInfo, LatencyRecorder, Resource, RunResult,
    Throughput, Variance,
};
use crate::workloads::{self, RunContext};

const HARNESS_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The cqlite tag this harness is pinned to (SPEC §2). Surfaced in every
/// result so reports and cross-version diffs are unambiguous.
const CQLITE_VERSION: &str = "v0.10.0";

/// Run one workload through the full trial protocol and produce a `RunResult`.
pub async fn run(name: &str, ctx: &RunContext) -> anyhow::Result<RunResult> {
    let mut trial_ops_per_sec: Vec<f64> = Vec::with_capacity(ctx.trials as usize);
    let mut trial_rows_per_sec: Vec<f64> = Vec::with_capacity(ctx.trials as usize);
    let mut merged = LatencyRecorder::new();
    let mut total_rows: u64 = 0;
    let mut peak_rss: u64 = 0;
    let mut cpu_means: Vec<f64> = Vec::new();
    // Static corpus facts (on-disk size, row count) for manifest-backed read
    // workloads; None for write/mixed. Captured once after the first setup.
    let mut dataset_meta: Option<workloads::DatasetMeta> = None;

    for trial in 0..ctx.trials {
        let mut boxed = workloads::build(name)?;
        boxed.setup(ctx).await?;
        if dataset_meta.is_none() {
            dataset_meta = boxed.dataset_meta();
        }

        if ctx.cold_cache {
            drop_page_cache();
        }

        let arc = Arc::new(boxed);

        // Warmup — results discarded (SPEC §9).
        if ctx.warmup_secs > 0 {
            let _ = drive(arc.clone(), ctx.concurrency, ctx.warmup_secs).await?;
        }

        // Measure, with resource sampling running alongside.
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(0));
        let cpu_samples = Arc::new(Mutex::new(Vec::<f64>::new()));
        let sampler = spawn_sampler(stop.clone(), peak.clone(), cpu_samples.clone());

        let (rec, ops, rows, elapsed) =
            drive(arc.clone(), ctx.concurrency, ctx.duration_secs).await?;

        stop.store(true, Ordering::Relaxed);
        let _ = sampler.await;

        let ops_per_sec = ops as f64 / elapsed.as_secs_f64();
        let rows_per_sec = rows as f64 / elapsed.as_secs_f64();
        trial_ops_per_sec.push(ops_per_sec);
        trial_rows_per_sec.push(rows_per_sec);
        merged.merge(&rec);
        total_rows += rows;
        peak_rss = peak_rss.max(peak.load(Ordering::Relaxed));
        let samples = cpu_samples.lock().await;
        if !samples.is_empty() {
            cpu_means.push(samples.iter().sum::<f64>() / samples.len() as f64);
        }
        drop(samples);

        let mut boxed = Arc::try_unwrap(arc)
            .map_err(|_| anyhow::anyhow!("workers still hold the workload after measurement"))?;
        boxed.teardown().await?;

        eprintln!(
            "  trial {}/{}: {:.0} ops/sec ({} ops, {} rows)",
            trial + 1,
            ctx.trials,
            ops_per_sec,
            ops,
            rows
        );
    }

    let median_ops = median(&trial_ops_per_sec);
    let median_rows = median(&trial_rows_per_sec);
    let cv = coefficient_of_variation(&trial_ops_per_sec);
    let cpu_mean = if cpu_means.is_empty() {
        0.0
    } else {
        cpu_means.iter().sum::<f64>() / cpu_means.len() as f64
    };

    Ok(RunResult {
        workload: name.to_string(),
        cqlite_version: CQLITE_VERSION.to_string(),
        cqlite_git_sha: None,
        harness_version: HARNESS_VERSION.to_string(),
        dataset: DatasetInfo {
            tier: ctx.tier.clone(),
            // Corpus row count from the manifest when known; else rows touched
            // (write/mixed, which have no manifest).
            rows: dataset_meta.map(|m| m.rows).unwrap_or(total_rows),
            bytes: dataset_meta.map(|m| m.bytes).unwrap_or(0),
            codec: ctx.codec.clone(),
            schema: ctx.schema.clone(),
        },
        distribution: ctx.distribution.clone(),
        concurrency: ctx.concurrency,
        cache: if ctx.cold_cache { "cold" } else { "warm" }.to_string(),
        throughput: Throughput {
            ops_per_sec: median_ops,
            // Rows touched per second, measured directly from each op's returned
            // row count — distinct from ops/sec for scans (many rows per op).
            rows_per_sec: median_rows,
            mb_per_sec: None,
        },
        latency_us: merged.snapshot(),
        resource: Resource {
            peak_rss_bytes: peak_rss,
            cpu_pct_mean: cpu_mean,
            alloc_bytes: None,
            alloc_count: None,
        },
        trials: ctx.trials,
        variance: Variance { ops_per_sec_cv: cv },
        host: HostInfo::detect(),
        duration_secs: ctx.duration_secs,
        warmup_secs: ctx.warmup_secs,
        seed: ctx.seed,
    })
}

/// Drive `concurrency` closed-loop workers for `secs`, each looping `op()` and
/// recording latency. Returns the merged histogram, total ops, total rows, and
/// the wall-clock elapsed.
async fn drive(
    wl: Arc<Box<dyn workloads::Workload>>,
    concurrency: usize,
    secs: u64,
) -> anyhow::Result<(LatencyRecorder, u64, u64, Duration)> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(concurrency.max(1));

    for w in 0..concurrency.max(1) {
        let wl = wl.clone();
        handles.push(tokio::spawn(async move {
            let mut rec = LatencyRecorder::new();
            let mut ops: u64 = 0;
            let mut rows: u64 = 0;
            while Instant::now() < deadline {
                let start = Instant::now();
                let r = wl.op(w).await?;
                let us = start.elapsed().as_micros() as u64;
                rec.record_micros(us);
                ops += 1;
                rows += r;
            }
            Ok::<(LatencyRecorder, u64, u64), anyhow::Error>((rec, ops, rows))
        }));
    }

    let mut merged = LatencyRecorder::new();
    let mut total_ops = 0u64;
    let mut total_rows = 0u64;
    for h in handles {
        let (rec, ops, rows) = h.await??;
        merged.merge(&rec);
        total_ops += ops;
        total_rows += rows;
    }

    Ok((merged, total_ops, total_rows, started.elapsed()))
}

/// Background RSS/CPU sampler (SPEC §5, step 5). Best-effort: failures to read
/// process stats are silently skipped rather than failing the run.
fn spawn_sampler(
    stop: Arc<AtomicBool>,
    peak_rss: Arc<AtomicU64>,
    cpu_samples: Arc<Mutex<Vec<f64>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut sys = System::new();
        let pid = Pid::from_u32(std::process::id());
        while !stop.load(Ordering::Relaxed) {
            sys.refresh_process(pid);
            if let Some(proc_) = sys.process(pid) {
                peak_rss.fetch_max(proc_.memory(), Ordering::Relaxed);
                cpu_samples.lock().await.push(proc_.cpu_usage() as f64);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
}

/// Cold-cache helper (SPEC §9). Invokes `scripts/drop-page-cache.sh` if present;
/// otherwise warns and continues (the run is reported as cold either way, so
/// the caller owns providing an honest environment).
fn drop_page_cache() {
    let script = std::path::Path::new("scripts/drop-page-cache.sh");
    if script.exists() {
        match std::process::Command::new("sh").arg(script).status() {
            Ok(s) if s.success() => {}
            Ok(s) => eprintln!("warning: drop-page-cache.sh exited with {s}"),
            Err(e) => eprintln!("warning: could not run drop-page-cache.sh: {e}"),
        }
    } else {
        eprintln!("warning: --cold-cache requested but scripts/drop-page-cache.sh is missing");
    }
}
