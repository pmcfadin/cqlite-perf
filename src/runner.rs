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
const CQLITE_VERSION: &str = "main@9054734";  // interim: post-#788/#790, pre-v0.12.0 tag

/// Run one workload through the full trial protocol and produce a `RunResult`.
pub async fn run(name: &str, ctx: &RunContext) -> anyhow::Result<RunResult> {
    let mut trial_ops_per_sec: Vec<f64> = Vec::with_capacity(ctx.trials as usize);
    let mut trial_rows_per_sec: Vec<f64> = Vec::with_capacity(ctx.trials as usize);
    let mut merged = LatencyRecorder::new();
    let mut total_rows: u64 = 0;
    let mut peak_rss: u64 = 0;
    let mut cpu_means: Vec<f64> = Vec::new();
    // Per-trial workload-specific metrics, aggregated (median per key) below.
    let mut custom_trials: Vec<std::collections::BTreeMap<String, f64>> = Vec::new();
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

        // A mixed workload supplies a cohort plan (heterogeneous roles, some
        // open-loop); everything else runs as a single closed-loop cohort.
        let plan_opt = arc.work_plan(ctx.concurrency);
        let plan = plan_opt
            .clone()
            .unwrap_or_else(|| workloads::WorkPlan::single("all", ctx.concurrency));
        let (cohorts, elapsed) = drive_plan(arc.clone(), &plan, ctx.duration_secs).await?;

        stop.store(true, Ordering::Relaxed);
        let _ = sampler.await;

        // Envelope = all cohorts merged; per-cohort detail goes to `custom`.
        let mut rec = LatencyRecorder::new();
        let mut ops: u64 = 0;
        let mut rows: u64 = 0;
        for c in &cohorts {
            rec.merge(&c.rec);
            ops += c.ops;
            rows += c.rows;
        }

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

        // Snapshot workload-specific metrics from the measurement phase before
        // teardown (op() accumulates them via interior mutability), plus — for a
        // mixed workload — per-cohort latency/throughput (read p99 under write
        // load, CO-corrected write p99, achieved vs target rate).
        let mut cm = arc.custom_metrics();
        if plan_opt.is_some() {
            for c in &cohorts {
                let snap = c.rec.snapshot();
                cm.insert(format!("{}.p50_us", c.label), snap.p50 as f64);
                cm.insert(format!("{}.p99_us", c.label), snap.p99 as f64);
                cm.insert(format!("{}.p999_us", c.label), snap.p999 as f64);
                cm.insert(
                    format!("{}.ops_per_sec", c.label),
                    c.ops as f64 / elapsed.as_secs_f64(),
                );
            }
        }
        if !cm.is_empty() {
            custom_trials.push(cm);
        }

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

    // Aggregate custom metrics: median across trials, per key. Keys are unioned
    // so a metric emitted in only some trials still reports (over those trials).
    let mut custom: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    if !custom_trials.is_empty() {
        let keys: std::collections::BTreeSet<String> = custom_trials
            .iter()
            .flat_map(|m| m.keys().cloned())
            .collect();
        for k in keys {
            let vals: Vec<f64> = custom_trials.iter().filter_map(|m| m.get(&k).copied()).collect();
            custom.insert(k, median(&vals));
        }
    }

    // dhat heap profiling (issue #4): emit the live-heap high-water (`t-gmax`)
    // as `custom.mem.live_heap_bytes` so the scorecard can gate memory on real
    // live allocation instead of sysinfo `peak_rss` (which v0.11.0's mmap read
    // path inflates ~11×, see cqlite-perf #5). `max_bytes` is the process-global
    // peak, so this is only meaningful for a single-workload dhat run, e.g.
    //   cargo run --release --features dhat-heap -- run --workload read.full_scan
    #[cfg(feature = "dhat-heap")]
    {
        let stats = dhat::HeapStats::get();
        custom.insert("mem.live_heap_bytes".to_string(), stats.max_bytes as f64);
    }

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
        custom,
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

/// Per-cohort measurement result: merged latency histogram + op/row totals for
/// one role-group of a mixed workload (or the lone "all" cohort otherwise).
struct CohortResult {
    label: String,
    rec: LatencyRecorder,
    ops: u64,
    rows: u64,
}

/// Drive a [`workloads::WorkPlan`]: each cohort runs its workers under its own
/// strategy — closed-loop, or open-loop at a target rate with coordinated-
/// omission correction (SPEC §5/§6). Worker ids are handed out in plan order so
/// `op(worker)` can dispatch by role. Returns one [`CohortResult`] per cohort
/// plus the wall-clock elapsed.
///
/// Open-loop cohorts split the cohort's target rate evenly across their workers;
/// each worker paces to its own fixed schedule and records latency from the
/// **intended** issue time, so a backed-up system (e.g. a flush stall) surfaces
/// as tail latency instead of being hidden by a slower issue cadence.
async fn drive_plan(
    wl: Arc<Box<dyn workloads::Workload>>,
    plan: &workloads::WorkPlan,
    secs: u64,
) -> anyhow::Result<(Vec<CohortResult>, Duration)> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let started = Instant::now();

    let mut handles = Vec::new();
    let mut next_id = 0usize;
    for cohort in &plan.cohorts {
        let workers = cohort.workers.max(if cohort.workers == 0 { 0 } else { 1 });
        for _ in 0..workers {
            let id = next_id;
            next_id += 1;
            let wl = wl.clone();
            let label = cohort.label.to_string();
            let rate = cohort.open_loop_rate;
            let cohort_workers = cohort.workers.max(1);
            handles.push(tokio::spawn(async move {
                let mut rec = LatencyRecorder::new();
                let mut ops: u64 = 0;
                let mut rows: u64 = 0;
                match rate {
                    // Closed-loop: next op as soon as the prior returns.
                    None => {
                        while Instant::now() < deadline {
                            let start = Instant::now();
                            let r = wl.op(id).await?;
                            rec.record_micros(start.elapsed().as_micros() as u64);
                            ops += 1;
                            rows += r;
                        }
                    }
                    // Open-loop at this worker's share of the cohort rate, with
                    // coordinated-omission correction.
                    Some(total_rate) => {
                        let per_worker = (total_rate / cohort_workers as f64).max(f64::MIN_POSITIVE);
                        let interval = Duration::from_secs_f64(1.0 / per_worker);
                        let t0 = Instant::now();
                        let mut slot: u32 = 0;
                        loop {
                            let intended = t0 + interval.saturating_mul(slot);
                            if intended >= deadline {
                                break;
                            }
                            let now = Instant::now();
                            if now < intended {
                                tokio::time::sleep(intended - now).await;
                            }
                            let r = wl.op(id).await?;
                            // Latency from the INTENDED issue time, not the actual
                            // start — coordinated-omission correction.
                            rec.record_micros(intended.elapsed().as_micros() as u64);
                            ops += 1;
                            rows += r;
                            slot = slot.saturating_add(1);
                        }
                    }
                }
                Ok::<(String, LatencyRecorder, u64, u64), anyhow::Error>((label, rec, ops, rows))
            }));
        }
    }

    // Merge per-worker results into per-cohort (by label, preserving order).
    let mut order: Vec<String> = Vec::new();
    let mut by_label: std::collections::HashMap<String, CohortResult> =
        std::collections::HashMap::new();
    for h in handles {
        let (label, rec, ops, rows) = h.await??;
        match by_label.get_mut(&label) {
            Some(c) => {
                c.rec.merge(&rec);
                c.ops += ops;
                c.rows += rows;
            }
            None => {
                order.push(label.clone());
                by_label.insert(
                    label.clone(),
                    CohortResult { label, rec, ops, rows },
                );
            }
        }
    }
    let cohorts = order
        .into_iter()
        .filter_map(|l| by_label.remove(&l))
        .collect();
    Ok((cohorts, started.elapsed()))
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
