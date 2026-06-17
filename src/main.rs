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
mod scorecard;
mod workloads;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::config::{parse_concurrency, parse_secs, RunConfig};
use crate::workloads::RunContext;

// dhat heap profiling (issue #4): swap the global allocator and emit
// dhat-heap.json on exit. Live-heap high-water (`At t-gmax`) is the number to
// compare against the sysinfo `peak_rss_bytes` the scorecard reports — they
// diverge when RSS is dominated by mmap/allocator-retained pages rather than
// live allocation. Gated so normal runs keep the system allocator.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

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
    /// Judge results against goals.toml and emit SCORECARD.md (PRD addendum US-2).
    Scorecard(ScorecardArgs),
    /// Re-render SUMMARY.md + SCORECARD.md from an existing results.jsonl without
    /// re-running — e.g. after several `run` invocations appended to one report
    /// dir (SPEC §11).
    Report(ReportArgs),
    /// Correctness gate: assert known-good row counts against the local corpora
    /// (TEST_PLAN.md Reference B). Exits non-zero on any mismatch. Run this
    /// before perf when validating a new engine version.
    Validate(ValidateArgs),
}

#[derive(Parser)]
struct ValidateArgs {
    /// Root holding the read corpora (read-<schema>-S-lz4 subdirs).
    #[arg(long, default_value = "datasets")]
    datasets_root: PathBuf,
}

#[derive(Parser)]
struct ReportArgs {
    /// results.jsonl to render (SUMMARY + SCORECARD written next to it).
    #[arg(long)]
    results: PathBuf,
    /// Declared goals file for the scorecard.
    #[arg(long, default_value = "goals.toml")]
    goals: PathBuf,
}

#[derive(Parser)]
struct ScorecardArgs {
    /// results.jsonl to judge (the current run).
    #[arg(long)]
    results: PathBuf,
    /// Declared goals file.
    #[arg(long, default_value = "goals.toml")]
    goals: PathBuf,
    /// Optional baseline results.jsonl for regression flagging.
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// Where to write SCORECARD.md (defaults next to --results).
    #[arg(long)]
    out: Option<PathBuf>,
    /// Exit non-zero if any enforced goal fails (for CI gating).
    #[arg(long, default_value_t = false)]
    enforce: bool,
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
    // Hold the profiler for the whole process; dropping it writes dhat-heap.json
    // and prints the Total / t-gmax / t-end summary.
    #[cfg(feature = "dhat-heap")]
    let _dhat = dhat::Profiler::new_heap();

    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => run(args).await,
        Command::Gen(args) => gen(args),
        Command::Datasets(args) => datasets_cmd(args),
        Command::Scorecard(args) => scorecard_cmd(args),
        Command::Report(args) => report_cmd(args),
        Command::Validate(args) => validate_cmd(args).await,
    }
}

/// Correctness gate (TEST_PLAN.md Reference B): ingest each local corpus and
/// assert the known-good row counts. A present corpus that returns the wrong
/// count fails the run (non-zero exit); an absent corpus is skipped with a
/// warning so the gate still runs in a partial dev checkout.
async fn validate_cmd(args: ValidateArgs) -> anyhow::Result<()> {
    use cqlite_core::ingestion::{ingest, IngestionConfig};
    use cqlite_core::query::result::StreamingConfig;
    use cqlite_core::{Config, Database};

    async fn count(db: &Database, q: &str) -> anyhow::Result<i64> {
        let mut it = db.execute_streaming(q, StreamingConfig::default()).await?;
        let mut n = 0i64;
        while let Some(r) = it.next_async().await {
            r.map_err(|e| anyhow::anyhow!("row error after {n}: {e}"))?;
            n += 1;
        }
        Ok(n)
    }

    // (schema file, corpus subdir, [(query, expected_rows, guards)]).
    let corpora: &[(&str, &str, &[(&str, i64, &str)])] = &[
        (
            "schemas/basic.cql",
            "read-basic-S-lz4",
            &[
                ("SELECT * FROM perf.basic", 100_000, "scan"),
                ("SELECT * FROM perf.basic WHERE id = 'k0000000000000000'", 1, "TEXT PK eq (#586)"),
                ("SELECT * FROM perf.basic LIMIT 3", 3, "LIMIT"),
                ("SELECT * FROM perf.basic WHERE age = 0", 834, "regular-col filter"),
                ("SELECT * FROM perf.basic WHERE name = 'name-0'", 1, "regular-col eq"),
            ],
        ),
        (
            "schemas/wide_rows.cql",
            "read-wide_rows-S-lz4",
            &[
                ("SELECT * FROM perf.wide_rows WHERE pk = 'p0000'", 1000, "partition restriction"),
                ("SELECT * FROM perf.wide_rows WHERE pk = 'p0000' AND ck >= 0 AND ck < 200", 200, "clustering bounds (#788)"),
                ("SELECT * FROM perf.wide_rows WHERE pk = 'p0000' AND ck < 200", 200, "open lower bound (#788)"),
                ("SELECT * FROM perf.wide_rows WHERE pk = 'p0000' AND ck >= 800", 200, "open upper bound (#788)"),
                ("SELECT * FROM perf.wide_rows WHERE pk = 'p0000' AND ck BETWEEN 0 AND 199", 200, "inclusive range"),
            ],
        ),
        (
            "schemas/collections.cql",
            "read-collections-S-lz4",
            &[("SELECT * FROM perf.collections", 100_000, "scan")],
        ),
    ];

    let root = std::env::current_dir()?;
    let mut checked = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;

    for (schema, subdir, checks) in corpora {
        let data_dir = args.datasets_root.join(subdir);
        if !data_dir.exists() {
            println!("⊘ {subdir}: corpus absent — skipped");
            skipped += 1;
            continue;
        }
        let cfg = IngestionConfig {
            schema_paths: vec![root.join(schema)],
            data_dir: data_dir.clone(),
            version_hint: Some("5.0".to_string()),
            core_config: Config::default(),
            table_directory_filter: None,
        };
        let db = match ingest(cfg).await {
            Ok(r) => r.database,
            Err(e) => {
                println!("✗ {subdir}: ingest failed: {e}");
                failed += 1;
                continue;
            }
        };
        println!("• {subdir}");
        for (q, expected, guards) in *checks {
            checked += 1;
            match count(&db, q).await {
                Ok(got) if got == *expected => {
                    println!("  ✅ {got:>6} (= {expected})  {guards}");
                }
                Ok(got) => {
                    failed += 1;
                    println!("  ❌ {got:>6} (≠ {expected})  {guards}  :: {q}");
                }
                Err(e) => {
                    failed += 1;
                    println!("  ❌ ERROR {guards}  :: {q}  :: {e}");
                }
            }
        }
    }

    println!("\n{checked} checks, {failed} failed, {skipped} corpus(es) skipped");
    if failed > 0 {
        anyhow::bail!("correctness gate FAILED — {failed} mismatch(es)");
    }
    if checked == 0 {
        anyhow::bail!("no corpora present — nothing validated (run `cqlite-perf gen` first)");
    }
    println!("✓ correctness gate passed");
    Ok(())
}

/// Re-render SUMMARY.md + SCORECARD.md from an existing results.jsonl (SPEC §11).
fn report_cmd(args: ReportArgs) -> anyhow::Result<()> {
    let results = scorecard::load_results(&args.results)?;
    if results.is_empty() {
        anyhow::bail!("{} has no results", args.results.display());
    }
    let dir = args
        .results
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));

    let summary = report::render_summary(&results);
    std::fs::write(dir.join("SUMMARY.md"), &summary)?;
    println!("✓ summary written to {}", dir.join("SUMMARY.md").display());

    if args.goals.exists() {
        let goals = scorecard::load_goals(&args.goals)?;
        let judgements = scorecard::judge(&goals, &results, &[]);
        let md = scorecard::render(&judgements, &results, &[]);
        std::fs::write(dir.join("SCORECARD.md"), md)?;
        println!("✓ scorecard written to {}", dir.join("SCORECARD.md").display());
        let (_, enforced_failed) = scorecard::tally(&judgements);
        if enforced_failed > 0 {
            println!("  ⚠ {enforced_failed} enforced goal(s) failed");
        }
    }
    Ok(())
}

/// Judge a results.jsonl against goals.toml and emit SCORECARD.md (US-2).
fn scorecard_cmd(args: ScorecardArgs) -> anyhow::Result<()> {
    let goals = scorecard::load_goals(&args.goals)?;
    let results = scorecard::load_results(&args.results)?;
    let baseline = match &args.baseline {
        Some(p) => scorecard::load_results(p)?,
        None => Vec::new(),
    };

    let judgements = scorecard::judge(&goals, &results, &baseline);
    let md = scorecard::render(&judgements, &results, &baseline);

    let out = args
        .out
        .unwrap_or_else(|| {
            args.results
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("SCORECARD.md")
        });
    std::fs::write(&out, &md)?;
    println!("✓ scorecard written to {}", out.display());
    print!("{md}");

    let (_, enforced_failed) = scorecard::tally(&judgements);
    if args.enforce && enforced_failed > 0 {
        anyhow::bail!("{enforced_failed} enforced goal(s) failed");
    }
    Ok(())
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
                        // Read workloads are bound to their corpus schema; honor
                        // that so the whole suite runs in one invocation. Other
                        // workloads use the caller's --schema.
                        let schema = crate::workloads::default_schema(name)
                            .map(str::to_string)
                            .unwrap_or_else(|| args.schema.clone());
                        let ctx = RunContext {
                            tier: tier.clone(),
                            schema,
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

    // Auto-emit the goals scorecard when goals.toml is present, so the
    // dev-team-facing report (US-2) is produced alongside the user-facing
    // SUMMARY without a separate command. Non-fatal if goals.toml is absent.
    let goals_path = std::path::Path::new("goals.toml");
    if goals_path.exists() && !all_results.is_empty() {
        match scorecard::load_goals(goals_path) {
            Ok(goals) => {
                let judgements = scorecard::judge(&goals, &all_results, &[]);
                let md = scorecard::render(&judgements, &all_results, &[]);
                let sc = report.dir().join("SCORECARD.md");
                std::fs::write(&sc, md)?;
                let (_, enforced_failed) = scorecard::tally(&judgements);
                println!("✓ scorecard written to {}", sc.display());
                if enforced_failed > 0 {
                    println!("  ⚠ {enforced_failed} enforced goal(s) failed");
                }
            }
            Err(e) => eprintln!("warning: could not load goals.toml: {e}"),
        }
    }
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
