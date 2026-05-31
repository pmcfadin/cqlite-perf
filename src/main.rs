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
    /// Generate a dataset corpus (SPEC §12). `--source cassandra` shells out to
    /// scripts/gen-corpus.sh.
    Gen(GenArgs),
    /// List/validate cached datasets against their manifests (SPEC §12).
    Datasets(DatasetsArgs),
}

#[derive(Parser)]
struct GenArgs {
    /// Generator source: "cassandra" (authentic read corpus) — "writer" lands in M2.
    #[arg(long, default_value = "cassandra")]
    source: String,
    #[arg(long, default_value = "S")]
    tier: String,
    #[arg(long, default_value = "lz4")]
    codec: String,
    #[arg(long, default_value = "basic")]
    schema: String,
}

#[derive(Parser)]
struct DatasetsArgs {
    /// Verify each cached dataset's SHA-256 against its manifest.
    #[arg(long)]
    check: bool,
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
        Command::Gen(args) => gen(args),
        Command::Datasets(args) => datasets_cmd(args),
    }
}

/// Generate a dataset by shelling out to the generator script (SPEC §8.1).
fn gen(args: GenArgs) -> anyhow::Result<()> {
    if args.source != "cassandra" {
        anyhow::bail!("gen --source '{}' not supported yet (M1: cassandra)", args.source);
    }
    let status = std::process::Command::new("bash")
        .arg("scripts/gen-corpus.sh")
        .args(["--tier", &args.tier, "--codec", &args.codec, "--schema", &args.schema])
        .status()?;
    if !status.success() {
        anyhow::bail!("gen-corpus.sh failed with {status}");
    }
    Ok(())
}

/// Validate cached datasets against their manifests (SPEC §8.3).
fn datasets_cmd(args: DatasetsArgs) -> anyhow::Result<()> {
    let repo_root = std::env::current_dir()?;
    let manifests_dir = repo_root.join("datasets/manifests");
    if !manifests_dir.is_dir() {
        println!("no manifests directory at {}", manifests_dir.display());
        return Ok(());
    }
    let mut ok = 0;
    let mut bad = 0;
    let mut missing = 0;
    for entry in std::fs::read_dir(&manifests_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let m = datasets::Manifest::from_path(&path)?;
        let dir = datasets::data_dir(&repo_root, &m);
        if !dir.is_dir() {
            println!("MISSING  {} (no {})", m.id, dir.display());
            missing += 1;
            continue;
        }
        if args.check {
            match datasets::compute_data_sha256(&dir) {
                Ok(sha) if sha == m.sha256 => {
                    println!("OK       {}", m.id);
                    ok += 1;
                }
                Ok(sha) => {
                    println!("MISMATCH {} (manifest {}… != {}…)", m.id, &m.sha256[..8.min(m.sha256.len())], &sha[..8]);
                    bad += 1;
                }
                Err(e) => {
                    println!("ERROR    {} ({e})", m.id);
                    bad += 1;
                }
            }
        } else {
            println!("FOUND    {} ({})", m.id, dir.display());
            ok += 1;
        }
    }
    println!("\n{ok} ok, {bad} bad, {missing} missing");
    if bad > 0 {
        anyhow::bail!("{bad} dataset(s) failed validation");
    }
    Ok(())
}

/// The fully-resolved run matrix, from either a config file or ad-hoc flags.
struct RunPlan {
    workloads: Vec<String>,
    concurrency: Vec<usize>,
    tiers: Vec<String>,
    codecs: Vec<String>,
    distributions: Vec<String>,
    warmup_secs: u64,
    duration_secs: u64,
    trials: u32,
    cold_cache: bool,
    seed: u64,
}

async fn run(args: RunArgs) -> anyhow::Result<()> {
    // Resolve the run matrix, either from a config file (full sweep over
    // tiers × codecs × distributions) or from ad-hoc flags (single axis values).
    let plan = if let Some(ref path) = args.config {
        let cfg = RunConfig::from_path(path)?;
        RunPlan {
            workloads: cfg.workload_names(),
            concurrency: cfg.concurrency.clone(),
            tiers: cfg.tiers.clone(),
            codecs: cfg.codecs.clone(),
            distributions: cfg.distributions.clone(),
            warmup_secs: parse_secs(&cfg.warmup)?,
            duration_secs: parse_secs(&cfg.duration)?,
            trials: cfg.trials,
            cold_cache: cfg.cold_cache,
            seed: cfg.seed,
        }
    } else {
        let wl = args
            .workload
            .clone()
            .ok_or_else(|| anyhow::anyhow!("provide either --config or --workload"))?;
        RunPlan {
            workloads: vec![wl],
            concurrency: parse_concurrency(&args.concurrency)?,
            tiers: vec![args.tier.clone()],
            codecs: vec![args.codec.clone()],
            distributions: vec![args.distribution.clone()],
            warmup_secs: parse_secs(&args.warmup)?,
            duration_secs: parse_secs(&args.duration)?,
            trials: args.trials,
            cold_cache: args.cold_cache,
            seed: args.seed,
        }
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

    // Sweep the full matrix: workload × tier × codec × distribution × concurrency
    // (SPEC §10 codec sweep, §6 distribution knob). Read workloads resolve their
    // dataset per (tier, codec) via manifest query; a missing dataset is reported
    // and skipped so one gap doesn't abort the whole sweep.
    let mut all_results = Vec::new();
    for name in &plan.workloads {
        for tier in &plan.tiers {
            for codec in &plan.codecs {
                for distribution in &plan.distributions {
                    for &conc in &plan.concurrency {
                        let ctx = RunContext {
                            tier: tier.clone(),
                            schema: args.schema.clone(),
                            codec: codec.clone(),
                            distribution: distribution.clone(),
                            concurrency: conc,
                            warmup_secs: plan.warmup_secs,
                            duration_secs: plan.duration_secs,
                            trials: plan.trials,
                            seed: plan.seed,
                            cold_cache: plan.cold_cache,
                            work_dir: work_dir.clone(),
                        };
                        println!(
                            "\n▶ {name}  (tier={tier} codec={codec} dist={distribution} \
                             conc={conc}, {}s × {} trials)",
                            plan.duration_secs, plan.trials
                        );
                        match runner::run(name, &ctx).await {
                            Ok(result) => {
                                report.append(&result)?;
                                all_results.push(result.clone());
                                println!(
                                    "  → {:.0} ops/sec  p50={}µs p99={}µs p999={}µs  cv={:.3}",
                                    result.throughput.ops_per_sec,
                                    result.latency_us.p50,
                                    result.latency_us.p99,
                                    result.latency_us.p999,
                                    result.variance.ops_per_sec_cv,
                                );
                            }
                            Err(e) => {
                                eprintln!("  ! skipped: {e}");
                            }
                        }
                    }
                }
            }
        }
    }

    report.write_summary(&all_results)?;
    println!("\n✓ results written to {}", report.jsonl_path().display());
    println!("✓ summary written to {}", report.dir().join("SUMMARY.md").display());
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
