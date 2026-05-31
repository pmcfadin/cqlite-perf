//! cqlite-perf — external macro-benchmark harness for CQLite (SPEC §12).
//!
//! M0 ships the `run` command driving `write.ingest` end-to-end: warmup →
//! duration-based measurement → trials → HDR latency → JSONL. The full
//! workload/dataset matrix, the `gen`/`compare`/`datasets` commands, and the
//! Markdown report land in M1+.

mod config;
mod datasets;
mod distribution;
mod metrics;
mod report;
mod runner;
mod workloads;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::config::{parse_concurrency, parse_secs, RunConfig};
use crate::workloads::RunContext;

#[derive(Parser)]
#[command(name = "cqlite-perf", version, about = "CQLite macro-benchmark harness")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a benchmark, either from a named config or ad hoc flags (SPEC §12).
    Run(RunArgs),
}

#[derive(Parser)]
struct RunArgs {
    /// Path to a named run config (TOML). Takes precedence over ad-hoc flags
    /// for workload/concurrency selection.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Single workload to run ad hoc, e.g. "write.ingest".
    #[arg(long)]
    workload: Option<String>,

    #[arg(long, default_value = "S")]
    tier: String,

    #[arg(long, default_value = "lz4")]
    codec: String,

    #[arg(long, default_value = "basic")]
    schema: String,

    #[arg(long, default_value = "zipfian")]
    distribution: String,

    /// Comma-separated concurrency levels, e.g. "1,2,4,8".
    #[arg(long, default_value = "1")]
    concurrency: String,

    #[arg(long, default_value = "5s")]
    warmup: String,

    #[arg(long, default_value = "10s")]
    duration: String,

    #[arg(long, default_value_t = 3)]
    trials: u32,

    #[arg(long, default_value_t = 42)]
    seed: u64,

    #[arg(long, default_value_t = false)]
    cold_cache: bool,

    /// Output root for reports.
    #[arg(long, default_value = "reports")]
    out: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => run(args).await,
    }
}

async fn run(args: RunArgs) -> anyhow::Result<()> {
    // Resolve the workload list and run parameters, either from a config file
    // or from ad-hoc flags.
    let (workloads, concurrency, warmup_secs, duration_secs, trials, cold_cache, seed) =
        if let Some(ref path) = args.config {
            let cfg = RunConfig::from_path(path)?;
            (
                cfg.workload_names(),
                cfg.concurrency.clone(),
                parse_secs(&cfg.warmup)?,
                parse_secs(&cfg.duration)?,
                cfg.trials,
                cfg.cold_cache,
                cfg.seed,
            )
        } else {
            let wl = args
                .workload
                .clone()
                .ok_or_else(|| anyhow::anyhow!("provide either --config or --workload"))?;
            (
                vec![wl],
                parse_concurrency(&args.concurrency)?,
                parse_secs(&args.warmup)?,
                parse_secs(&args.duration)?,
                args.trials,
                args.cold_cache,
                args.seed,
            )
        };

    let work_dir = std::env::temp_dir().join("cqlite-perf");
    std::fs::create_dir_all(&work_dir)?;

    let date = today();
    let report = report::Report::create(&args.out, &date, env!("CARGO_PKG_VERSION"))?;

    println!(
        "cqlite-perf {} → {}",
        env!("CARGO_PKG_VERSION"),
        report.jsonl_path().display()
    );

    for name in &workloads {
        for &conc in &concurrency {
            let ctx = RunContext {
                tier: args.tier.clone(),
                schema: args.schema.clone(),
                codec: args.codec.clone(),
                distribution: args.distribution.clone(),
                concurrency: conc,
                warmup_secs,
                duration_secs,
                trials,
                seed,
                cold_cache,
                work_dir: work_dir.clone(),
            };
            println!("\n▶ {name}  (concurrency={conc}, {duration_secs}s × {trials} trials)");
            let result = runner::run(name, &ctx).await?;
            report.append(&result)?;
            println!(
                "  → {:.0} ops/sec  p50={}µs p99={}µs p999={}µs  cv={:.3}",
                result.throughput.ops_per_sec,
                result.latency_us.p50,
                result.latency_us.p99,
                result.latency_us.p999,
                result.variance.ops_per_sec_cv,
            );
        }
    }

    println!("\n✓ results written to {}", report.jsonl_path().display());
    Ok(())
}

/// Today's date as YYYY-MM-DD for the report directory name. Best-effort via the
/// system `date`; falls back to "undated" so a missing tool never fails a run.
fn today() -> String {
    std::process::Command::new("date")
        .arg("+%Y-%m-%d")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "undated".to_string())
}
