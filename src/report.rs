//! Output & reporting (SPEC §11). JSON/JSONL is the canonical record and the
//! input to regression diffing and cross-version comparison. The Markdown
//! SUMMARY emitter lands in M1 once there are multi-dimensional sweeps to
//! cross-tabulate.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::metrics::RunResult;

/// A run-scoped report directory: `reports/<date>-<version>/`.
pub struct Report {
    dir: PathBuf,
    jsonl: PathBuf,
}

impl Report {
    /// Create (or reuse) a report directory under `out_root`. The date is taken
    /// from the caller so the path is explicit rather than hidden wall-clock.
    pub fn create(out_root: &Path, date: &str, version: &str) -> anyhow::Result<Self> {
        let dir = out_root.join(format!("{date}-{version}"));
        std::fs::create_dir_all(&dir)?;
        let jsonl = dir.join("results.jsonl");
        Ok(Self { dir, jsonl })
    }

    /// Append one `RunResult` as a single JSON line (SPEC §11).
    pub fn append(&self, result: &RunResult) -> anyhow::Result<()> {
        let line = serde_json::to_string(result)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.jsonl)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    pub fn jsonl_path(&self) -> &Path {
        &self.jsonl
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Write `SUMMARY.md` with the headline tables (SPEC §11). Regenerated from
    /// the accumulated results each time, so it always reflects the full run.
    pub fn write_summary(&self, results: &[RunResult]) -> anyhow::Result<()> {
        let md = render_summary(results);
        std::fs::write(self.dir.join("SUMMARY.md"), md)?;
        Ok(())
    }
}

/// Render the headline Markdown tables from a set of results (SPEC §11):
/// throughput-by-concurrency and a codec sweep when more than one codec appears.
pub fn render_summary(results: &[RunResult]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();

    let version = results
        .first()
        .map(|r| r.cqlite_version.as_str())
        .unwrap_or("unknown");
    let host = results.first().map(|r| {
        format!(
            "{} ({} cores), {}",
            r.host.cpu, r.host.cores, r.host.os
        )
    });

    let _ = writeln!(s, "# cqlite-perf — run summary\n");
    let _ = writeln!(s, "- **cqlite:** {version}");
    if let Some(h) = host {
        let _ = writeln!(s, "- **host:** {h}");
    }
    let _ = writeln!(s, "- **results:** {} run(s)\n", results.len());

    // Throughput by concurrency, grouped by workload.
    let _ = writeln!(s, "## Throughput by concurrency\n");
    let _ = writeln!(
        s,
        "| Workload | Conc | Cache | ops/sec | rows/sec | p50 µs | p99 µs | p99.9 µs | CV |"
    );
    let _ = writeln!(
        s,
        "|---|---:|---|---:|---:|---:|---:|---:|---:|"
    );
    let mut sorted: Vec<&RunResult> = results.iter().collect();
    sorted.sort_by(|a, b| {
        a.workload
            .cmp(&b.workload)
            .then(a.concurrency.cmp(&b.concurrency))
    });
    for r in &sorted {
        let _ = writeln!(
            s,
            "| {} | {} | {} | {:.0} | {:.0} | {} | {} | {} | {:.3} |",
            r.workload,
            r.concurrency,
            r.cache,
            r.throughput.ops_per_sec,
            r.throughput.rows_per_sec,
            r.latency_us.p50,
            r.latency_us.p99,
            r.latency_us.p999,
            r.variance.ops_per_sec_cv,
        );
    }

    // Codec sweep (SPEC §10) — only meaningful with >1 codec present.
    let codecs: std::collections::BTreeSet<&str> =
        results.iter().map(|r| r.dataset.codec.as_str()).collect();
    if codecs.len() > 1 {
        let _ = writeln!(s, "\n## Codec sweep\n");
        let _ = writeln!(
            s,
            "| Codec | Tier | Size (bytes) | Scan rows/sec | p99 µs |"
        );
        let _ = writeln!(s, "|---|---|---:|---:|---:|");
        let mut sweep: Vec<&RunResult> = results
            .iter()
            .filter(|r| r.workload == "read.full_scan")
            .collect();
        sweep.sort_by(|a, b| a.dataset.codec.cmp(&b.dataset.codec));
        for r in &sweep {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {:.0} | {} |",
                r.dataset.codec,
                r.dataset.tier,
                r.dataset.bytes,
                r.throughput.rows_per_sec,
                r.latency_us.p99,
            );
        }
    }

    s
}
