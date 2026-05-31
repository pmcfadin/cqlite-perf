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
}
