//! Run config (TOML) parsing (SPEC §12) and shared parse helpers.

use serde::Deserialize;

/// A named run config file (SPEC §12, e.g. configs/headline.toml).
#[derive(Debug, Clone, Deserialize)]
pub struct RunConfig {
    pub warmup: String,
    pub duration: String,
    pub trials: u32,
    pub concurrency: Vec<usize>,
    #[serde(default = "default_distributions")]
    pub distributions: Vec<String>,
    #[serde(default = "default_codecs")]
    pub codecs: Vec<String>,
    #[serde(default = "default_tiers")]
    pub tiers: Vec<String>,
    #[serde(default)]
    pub cold_cache: bool,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default)]
    pub suite: Vec<Suite>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Suite {
    pub name: String,
    pub workloads: Vec<String>,
}

fn default_distributions() -> Vec<String> {
    vec!["zipfian".to_string()]
}
fn default_codecs() -> Vec<String> {
    vec!["lz4".to_string()]
}
fn default_tiers() -> Vec<String> {
    vec!["S".to_string()]
}
fn default_seed() -> u64 {
    42
}

impl RunConfig {
    pub fn from_path(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading config {}: {e}", path.display()))?;
        let cfg: RunConfig = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parsing config {}: {e}", path.display()))?;
        Ok(cfg)
    }

    /// Fully-qualified workload names across all suites, e.g. "write.ingest".
    pub fn workload_names(&self) -> Vec<String> {
        self.suite
            .iter()
            .flat_map(|s| s.workloads.iter().map(move |w| format!("{}.{}", s.name, w)))
            .collect()
    }
}

/// Parse a duration string like "5s", "30s", "2m" into whole seconds.
pub fn parse_secs(s: &str) -> anyhow::Result<u64> {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix("ms") {
        // Sub-second durations round down for the duration-based loop.
        let v: u64 = rest.trim().parse()?;
        return Ok(v / 1000);
    }
    let (num, mult) = if let Some(rest) = s.strip_suffix('s') {
        (rest, 1)
    } else if let Some(rest) = s.strip_suffix('m') {
        (rest, 60)
    } else if let Some(rest) = s.strip_suffix('h') {
        (rest, 3600)
    } else {
        (s, 1) // bare number → seconds
    };
    let v: u64 = num
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid duration '{s}': {e}"))?;
    Ok(v * mult)
}

/// Parse a comma-separated concurrency list like "1,2,4,8,16".
pub fn parse_concurrency(s: &str) -> anyhow::Result<Vec<usize>> {
    s.split(',')
        .map(|p| {
            p.trim()
                .parse::<usize>()
                .map_err(|e| anyhow::anyhow!("invalid concurrency '{p}': {e}"))
        })
        .collect()
}
