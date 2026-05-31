//! Goals scorecard (PRD addendum US-2): judge each `RunResult` against a
//! declared target in `goals.toml`, optionally against a baseline, and emit
//! `SCORECARD.md`. Exit non-zero when an enforced goal fails, so CI can gate.

use std::path::Path;

use serde::Deserialize;

use crate::metrics::RunResult;

/// One declared goal. Carries either an absolute `target` (with `op`) or a
/// `regression_max_pct` judged against a baseline — not both.
#[derive(Debug, Clone, Deserialize)]
pub struct Goal {
    pub name: String,
    pub metric: String,
    #[serde(default)]
    pub op: Option<String>,
    #[serde(default)]
    pub target: Option<f64>,
    #[serde(default)]
    pub regression_max_pct: Option<f64>,
    #[serde(default)]
    pub enforce: bool,

    // Selectors (omitted = matches any).
    #[serde(default)]
    pub workload: Option<String>,
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub distribution: Option<String>,
    #[serde(default)]
    pub concurrency: Option<usize>,
    #[serde(default)]
    pub cache: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoalsFile {
    #[serde(default)]
    goal: Vec<Goal>,
}

pub fn load_goals(path: &Path) -> anyhow::Result<Vec<Goal>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("reading goals {}: {e}", path.display()))?;
    let f: GoalsFile = toml::from_str(&text)
        .map_err(|e| anyhow::anyhow!("parsing goals {}: {e}", path.display()))?;
    Ok(f.goal)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Met,
    Missed,
    Regressed,
    Improved,
    NoData,
}

impl Status {
    fn badge(&self) -> &'static str {
        match self {
            Status::Met => "✅ MET",
            Status::Missed => "❌ MISSED",
            Status::Regressed => "❌ REGRESSED",
            Status::Improved => "✅ IMPROVED",
            Status::NoData => "— NO-DATA",
        }
    }
}

pub struct Judgement {
    pub goal: Goal,
    pub status: Status,
    pub actual: Option<f64>,
    pub baseline: Option<f64>,
}

impl Goal {
    fn matches(&self, r: &RunResult) -> bool {
        let sel = |g: &Option<String>, v: &str| g.as_deref().map_or(true, |x| x == v);
        sel(&self.workload, &r.workload)
            && sel(&self.schema, &r.dataset.schema)
            && sel(&self.tier, &r.dataset.tier)
            && sel(&self.codec, &r.dataset.codec)
            && sel(&self.distribution, &r.distribution)
            && sel(&self.cache, &r.cache)
            && self.concurrency.map_or(true, |c| c == r.concurrency)
    }
}

/// Extract a metric by its dotted path from a result's envelope.
fn metric_value(r: &RunResult, path: &str) -> Option<f64> {
    match path {
        "throughput.ops_per_sec" => Some(r.throughput.ops_per_sec),
        "throughput.rows_per_sec" => Some(r.throughput.rows_per_sec),
        "throughput.mb_per_sec" => r.throughput.mb_per_sec,
        "latency_us.p50" => Some(r.latency_us.p50 as f64),
        "latency_us.p90" => Some(r.latency_us.p90 as f64),
        "latency_us.p95" => Some(r.latency_us.p95 as f64),
        "latency_us.p99" => Some(r.latency_us.p99 as f64),
        "latency_us.p999" => Some(r.latency_us.p999 as f64),
        "latency_us.max" => Some(r.latency_us.max as f64),
        "resource.peak_rss_bytes" => Some(r.resource.peak_rss_bytes as f64),
        "resource.cpu_pct_mean" => Some(r.resource.cpu_pct_mean),
        _ => None,
    }
}

fn compare(actual: f64, op: &str, target: f64) -> bool {
    match op {
        ">=" => actual >= target,
        "<=" => actual <= target,
        ">" => actual > target,
        "<" => actual < target,
        "==" => (actual - target).abs() < f64::EPSILON,
        _ => false,
    }
}

/// Build a lookup of the best (first) matching baseline value per goal name.
fn baseline_value(goal: &Goal, baseline: &[RunResult]) -> Option<f64> {
    baseline
        .iter()
        .find(|r| goal.matches(r))
        .and_then(|r| metric_value(r, &goal.metric))
}

/// Judge every goal against the results (and optional baseline).
pub fn judge(goals: &[Goal], results: &[RunResult], baseline: &[RunResult]) -> Vec<Judgement> {
    goals
        .iter()
        .map(|g| {
            // The actual value is taken from the first matching result.
            let actual = results
                .iter()
                .find(|r| g.matches(r))
                .and_then(|r| metric_value(r, &g.metric));
            let base = baseline_value(g, baseline);

            let status = match (actual, g.regression_max_pct, g.target.as_ref(), g.op.as_deref())
            {
                (None, _, _, _) => Status::NoData,
                // Regression-budget goal: compare against baseline.
                (Some(a), Some(max_pct), _, _) => match base {
                    None => Status::NoData,
                    Some(b) if b == 0.0 => Status::NoData,
                    Some(b) => {
                        // "higher is better" metrics (throughput) regress when
                        // they drop; for latency/RSS a *rise* is the regression.
                        // We treat the budget symmetrically on the metric's
                        // natural direction inferred from the metric name.
                        let higher_better = g.metric.starts_with("throughput");
                        let pct_change = (a - b) / b * 100.0;
                        let worse_pct = if higher_better { -pct_change } else { pct_change };
                        if worse_pct > max_pct {
                            Status::Regressed
                        } else if worse_pct < 0.0 {
                            Status::Improved
                        } else {
                            Status::Met
                        }
                    }
                },
                // Absolute-target goal.
                (Some(a), None, Some(&t), Some(op)) => {
                    if compare(a, op, t) {
                        Status::Met
                    } else {
                        Status::Missed
                    }
                }
                _ => Status::NoData,
            };

            Judgement {
                goal: g.clone(),
                status,
                actual,
                baseline: base,
            }
        })
        .collect()
}

/// Render SCORECARD.md (PRD addendum §5).
pub fn render(judgements: &[Judgement], results: &[RunResult], baseline: &[RunResult]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();

    let ver = results.first().map(|r| r.cqlite_version.as_str()).unwrap_or("?");
    let base_ver = baseline.first().map(|r| r.cqlite_version.as_str());
    let host = results
        .first()
        .map(|r| format!("{} ({} cores)", r.host.cpu, r.host.cores))
        .unwrap_or_default();

    let _ = writeln!(s, "# cqlite-perf — goals scorecard\n");
    match base_ver {
        Some(b) => {
            let _ = writeln!(s, "cqlite **{ver}** · baseline **{b}** · {host}\n");
        }
        None => {
            let _ = writeln!(s, "cqlite **{ver}** · no baseline · {host}\n");
        }
    }

    let _ = writeln!(
        s,
        "| Goal | Metric | Target | Actual | vs base | Status | Enforced |"
    );
    let _ = writeln!(s, "|------|--------|-------:|-------:|--------:|--------|:--------:|");

    for j in judgements {
        let target = match (&j.goal.target, &j.goal.regression_max_pct, &j.goal.op) {
            (Some(t), _, Some(op)) => format!("{op} {}", fmt_num(*t)),
            (_, Some(p), _) => format!("≤{p}% regress"),
            _ => "—".to_string(),
        };
        let actual = j.actual.map(fmt_num).unwrap_or_else(|| "—".to_string());
        let vs_base = match (j.actual, j.baseline) {
            (Some(a), Some(b)) if b != 0.0 => format!("{:+.1}%", (a - b) / b * 100.0),
            _ => "—".to_string(),
        };
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} | {} | {} |",
            j.goal.name,
            j.goal.metric,
            target,
            actual,
            vs_base,
            j.status.badge(),
            if j.goal.enforce { "yes" } else { "no" },
        );
    }

    let (failed, enforced_failed) = tally(judgements);
    let _ = writeln!(
        s,
        "\n{} goal(s), {failed} failed ({enforced_failed} enforced).",
        judgements.len()
    );
    if enforced_failed > 0 {
        let _ = writeln!(s, "\n**{enforced_failed} enforced goal(s) failed → exit 1**");
    }
    s
}

fn fmt_num(v: f64) -> String {
    if v >= 1000.0 {
        // Thousands separator for readability.
        let i = v.round() as i64;
        let mut out = String::new();
        let digits = i.abs().to_string();
        let bytes = digits.as_bytes();
        for (idx, ch) in bytes.iter().enumerate() {
            if idx > 0 && (bytes.len() - idx) % 3 == 0 {
                out.push(',');
            }
            out.push(*ch as char);
        }
        if i < 0 {
            format!("-{out}")
        } else {
            out
        }
    } else {
        format!("{v:.0}")
    }
}

/// (total failed, enforced failed) — failures are Missed or Regressed.
pub fn tally(judgements: &[Judgement]) -> (usize, usize) {
    let mut failed = 0;
    let mut enforced_failed = 0;
    for j in judgements {
        let is_fail = matches!(j.status, Status::Missed | Status::Regressed);
        if is_fail {
            failed += 1;
            if j.goal.enforce {
                enforced_failed += 1;
            }
        }
    }
    (failed, enforced_failed)
}

/// Load a results.jsonl file into a Vec<RunResult>.
pub fn load_results(path: &Path) -> anyhow::Result<Vec<RunResult>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("reading results {}: {e}", path.display()))?;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let r: RunResult = serde_json::from_str(line)
            .map_err(|e| anyhow::anyhow!("parsing {} line {}: {e}", path.display(), i + 1))?;
        out.push(r);
    }
    Ok(out)
}
