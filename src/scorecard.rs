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
        // Workload-specific metrics (write/mixed), e.g. `custom.flush.mb_per_sec`.
        other => other
            .strip_prefix("custom.")
            .and_then(|k| r.custom.get(k).copied()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{DatasetInfo, HostInfo, LatencyUs, Resource, Throughput, Variance};
    use std::collections::BTreeMap;

    /// Build a minimal RunResult fixture for use in scorecard unit tests.
    fn make_result(
        workload: &str,
        schema: &str,
        tier: &str,
        rows_per_sec: f64,
        p99_us: u64,
    ) -> RunResult {
        RunResult {
            workload: workload.to_string(),
            cqlite_version: "test".to_string(),
            cqlite_git_sha: None,
            harness_version: "0.0.0".to_string(),
            dataset: DatasetInfo {
                tier: tier.to_string(),
                rows: 100_000,
                bytes: 1_000_000,
                codec: "lz4".to_string(),
                schema: schema.to_string(),
            },
            distribution: "zipfian".to_string(),
            concurrency: 1,
            cache: "warm".to_string(),
            throughput: Throughput {
                ops_per_sec: rows_per_sec,
                rows_per_sec,
                mb_per_sec: None,
            },
            latency_us: LatencyUs {
                p50: p99_us / 2,
                p90: p99_us,
                p95: p99_us,
                p99: p99_us,
                p999: p99_us * 2,
                max: p99_us * 3,
            },
            resource: Resource {
                peak_rss_bytes: 100_000_000,
                cpu_pct_mean: 50.0,
                alloc_bytes: None,
                alloc_count: None,
            },
            trials: 1,
            variance: Variance { ops_per_sec_cv: 0.01 },
            host: HostInfo {
                cpu: "x86_64".to_string(),
                cores: 4,
                ram_gb: 16,
                os: "linux".to_string(),
            },
            duration_secs: 10,
            warmup_secs: 5,
            seed: 42,
            custom: BTreeMap::new(),
        }
    }

    /// Build a regression_max_pct goal for testing.
    fn regression_goal(
        metric: &str,
        workload: &str,
        schema: Option<&str>,
        tier: Option<&str>,
        max_pct: f64,
        enforce: bool,
    ) -> Goal {
        Goal {
            name: format!("test {metric} regression"),
            metric: metric.to_string(),
            op: None,
            target: None,
            regression_max_pct: Some(max_pct),
            enforce,
            workload: Some(workload.to_string()),
            schema: schema.map(str::to_string),
            tier: tier.map(str::to_string),
            codec: None,
            distribution: None,
            concurrency: None,
            cache: None,
        }
    }

    // ── throughput regression (higher-is-better) ─────────────────────────────

    #[test]
    fn throughput_regression_above_budget_is_regressed() {
        // Baseline: 332,351 rows/s. Actual: 31% below (229,322) → exceeds 30% budget.
        let baseline_val = 332_351.0_f64;
        let actual_val = baseline_val * 0.69; // -31%
        let goal = regression_goal(
            "throughput.rows_per_sec",
            "read.full_scan",
            Some("basic"),
            Some("S"),
            30.0,
            true,
        );
        let actual = make_result("read.full_scan", "basic", "S", actual_val, 1_000);
        let baseline = make_result("read.full_scan", "basic", "S", baseline_val, 1_000);

        let js = judge(&[goal], &[actual], &[baseline]);
        assert_eq!(js.len(), 1);
        assert_eq!(js[0].status, Status::Regressed);

        let (failed, enforced_failed) = tally(&js);
        assert_eq!(failed, 1);
        assert_eq!(enforced_failed, 1, "enforced goal must count in enforced_failed");
    }

    #[test]
    fn throughput_regression_within_budget_is_met() {
        // Actual is 20% below baseline → within 30% budget.
        let baseline_val = 332_351.0_f64;
        let actual_val = baseline_val * 0.80; // -20%
        let goal = regression_goal(
            "throughput.rows_per_sec",
            "read.full_scan",
            Some("basic"),
            Some("S"),
            30.0,
            true,
        );
        let actual = make_result("read.full_scan", "basic", "S", actual_val, 1_000);
        let baseline = make_result("read.full_scan", "basic", "S", baseline_val, 1_000);

        let js = judge(&[goal], &[actual], &[baseline]);
        assert_eq!(js[0].status, Status::Met);
        let (_, enforced_failed) = tally(&js);
        assert_eq!(enforced_failed, 0);
    }

    #[test]
    fn throughput_improvement_is_improved() {
        // Actual is 20% ABOVE baseline → improvement, not regression.
        let baseline_val = 332_351.0_f64;
        let actual_val = baseline_val * 1.20; // +20%
        let goal = regression_goal(
            "throughput.rows_per_sec",
            "read.full_scan",
            Some("basic"),
            Some("S"),
            30.0,
            true,
        );
        let actual = make_result("read.full_scan", "basic", "S", actual_val, 1_000);
        let baseline = make_result("read.full_scan", "basic", "S", baseline_val, 1_000);

        let js = judge(&[goal], &[actual], &[baseline]);
        assert_eq!(js[0].status, Status::Improved);
        let (_, enforced_failed) = tally(&js);
        assert_eq!(enforced_failed, 0);
    }

    // ── no-baseline → NoData (not a failure) ─────────────────────────────────

    #[test]
    fn no_baseline_yields_no_data_not_failure() {
        // Actual present but no baseline results → NoData; must NOT count as failed.
        let goal = regression_goal(
            "throughput.rows_per_sec",
            "read.full_scan",
            Some("basic"),
            Some("S"),
            30.0,
            true,
        );
        let actual = make_result("read.full_scan", "basic", "S", 332_351.0, 1_000);

        let js = judge(&[goal], &[actual], &[]); // empty baseline
        assert_eq!(js[0].status, Status::NoData);

        let (failed, enforced_failed) = tally(&js);
        assert_eq!(failed, 0, "NoData must not be counted as failed");
        assert_eq!(enforced_failed, 0);
    }

    // ── latency p99 regression (lower-is-better) ─────────────────────────────

    #[test]
    fn latency_p99_regression_above_budget_is_regressed() {
        // Baseline p99: 401,151 µs. Actual: 31% above (525,508 µs) → exceeds 30% budget.
        let baseline_p99 = 401_151_u64;
        let actual_p99 = (baseline_p99 as f64 * 1.31).round() as u64; // +31%
        let goal = regression_goal(
            "latency_us.p99",
            "read.point_lookup",
            None,
            None,
            30.0,
            true,
        );
        let actual = make_result("read.point_lookup", "basic", "S", 1.0, actual_p99);
        let baseline = make_result("read.point_lookup", "basic", "S", 1.0, baseline_p99);

        let js = judge(&[goal], &[actual], &[baseline]);
        assert_eq!(js[0].status, Status::Regressed, "p99 rise beyond budget must be Regressed");

        let (failed, enforced_failed) = tally(&js);
        assert_eq!(failed, 1);
        assert_eq!(enforced_failed, 1);
    }

    #[test]
    fn latency_p99_within_budget_is_met() {
        // Actual p99 is 20% above baseline → within 30% budget.
        let baseline_p99 = 401_151_u64;
        let actual_p99 = (baseline_p99 as f64 * 1.20).round() as u64; // +20%
        let goal = regression_goal(
            "latency_us.p99",
            "read.point_lookup",
            None,
            None,
            30.0,
            true,
        );
        let actual = make_result("read.point_lookup", "basic", "S", 1.0, actual_p99);
        let baseline = make_result("read.point_lookup", "basic", "S", 1.0, baseline_p99);

        let js = judge(&[goal], &[actual], &[baseline]);
        assert_eq!(js[0].status, Status::Met);
        let (_, enforced_failed) = tally(&js);
        assert_eq!(enforced_failed, 0);
    }

    // ── tally: non-enforced regressions do not count as enforced failures ─────

    #[test]
    fn non_enforced_regression_not_counted_in_enforced_failed() {
        let baseline_val = 332_351.0_f64;
        let actual_val = baseline_val * 0.50; // −50% — would be a regression
        let goal = regression_goal(
            "throughput.rows_per_sec",
            "read.full_scan",
            Some("basic"),
            Some("S"),
            30.0,
            false, // NOT enforced
        );
        let actual = make_result("read.full_scan", "basic", "S", actual_val, 1_000);
        let baseline = make_result("read.full_scan", "basic", "S", baseline_val, 1_000);

        let js = judge(&[goal], &[actual], &[baseline]);
        assert_eq!(js[0].status, Status::Regressed);

        let (failed, enforced_failed) = tally(&js);
        assert_eq!(failed, 1, "non-enforced regression IS a failure in total count");
        assert_eq!(enforced_failed, 0, "non-enforced regression must NOT count in enforced_failed");
    }

    // ── soak trend goals (issue #32) ─────────────────────────────────────────

    /// Build an absolute-target (op/target) goal for testing, mirroring the
    /// soak trend goals in goals.toml.
    fn trend_goal(metric: &str, workload: &str, op: &str, target: f64) -> Goal {
        Goal {
            name: format!("test {metric}"),
            metric: metric.to_string(),
            op: Some(op.to_string()),
            target: Some(target),
            regression_max_pct: None,
            enforce: false,
            workload: Some(workload.to_string()),
            schema: None,
            tier: None,
            codec: None,
            distribution: None,
            concurrency: None,
            cache: None,
        }
    }

    #[test]
    fn trend_goal_on_non_soak_run_is_no_data_not_failed() {
        // A normal (non-soak) run has an EMPTY custom map — no `trend.*` keys
        // at all, since only `--snapshot-interval` runs populate them (#31).
        // This is the behavior that keeps ci.yml's `scorecard --enforce` safe
        // once these goals exist in the shared goals.toml: short smokes with
        // no --snapshot-interval must see NoData, never a spurious failure.
        let goal = trend_goal(
            "custom.trend.throughput_retention",
            "mixed.read_while_write",
            ">=",
            0.95,
        );
        let result = make_result("mixed.read_while_write", "basic", "S", 100_000.0, 200_000);
        assert!(result.custom.is_empty(), "non-soak fixture must have no custom keys");

        let js = judge(&[goal], &[result], &[]);
        assert_eq!(js.len(), 1);
        assert_eq!(js[0].status, Status::NoData);

        let (failed, enforced_failed) = tally(&js);
        assert_eq!(failed, 0, "NoData trend goal must not be counted as failed");
        assert_eq!(enforced_failed, 0);
    }

    #[test]
    fn trend_goal_met_when_value_satisfies_target() {
        let goal = trend_goal(
            "custom.trend.throughput_retention",
            "mixed.read_while_write",
            ">=",
            0.95,
        );
        let mut result = make_result("mixed.read_while_write", "basic", "S", 100_000.0, 200_000);
        result.custom.insert("trend.throughput_retention".to_string(), 0.97);

        let js = judge(&[goal], &[result], &[]);
        assert_eq!(js[0].status, Status::Met);
        assert_eq!(js[0].actual, Some(0.97));

        let (failed, _) = tally(&js);
        assert_eq!(failed, 0);
    }

    #[test]
    fn trend_goal_missed_when_value_violates_target() {
        let goal = trend_goal(
            "custom.trend.throughput_retention",
            "mixed.read_while_write",
            ">=",
            0.95,
        );
        let mut result = make_result("mixed.read_while_write", "basic", "S", 100_000.0, 200_000);
        result.custom.insert("trend.throughput_retention".to_string(), 0.80);

        let js = judge(&[goal], &[result], &[]);
        assert_eq!(js[0].status, Status::Missed);

        let (failed, enforced_failed) = tally(&js);
        assert_eq!(failed, 1);
        assert_eq!(enforced_failed, 0, "goal.enforce is false in this fixture");
    }
}
