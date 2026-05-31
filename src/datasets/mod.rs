//! Dataset manifest model (SPEC §8.3) and selection-by-query (SPEC §8).
//!
//! Binaries are never committed — only manifests + checksums. The harness
//! selects datasets by manifest query (tier + schema + codec), never by
//! hard-coded path, and verifies the cached binary's SHA-256 on run start.
//!
//! M0 defines the manifest type. The Cassandra generator (cassandra_gen.rs)
//! and writer generator (writer_gen.rs) land in M1/M2.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A dataset manifest (SPEC §8.3). Committed under `datasets/manifests/`; the
/// binary it describes is gitignored and regenerated/fetched on demand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub tier: String,
    pub schema: String,
    pub codec: String,
    pub rows: u64,
    pub bytes: u64,
    pub generator: String,
    pub generator_version: String,
    pub cqlite_schema_ref: String,
    pub sha256: String,
    pub created_utc: String,
    pub path_hint: String,
}

/// A manifest query (SPEC §8): selection is by these axes, never by path.
#[derive(Debug, Clone)]
pub struct ManifestQuery {
    pub tier: String,
    pub schema: String,
    pub codec: String,
}

impl Manifest {
    pub fn matches(&self, q: &ManifestQuery) -> bool {
        self.tier == q.tier && self.schema == q.schema && self.codec == q.codec
    }

    /// Load a manifest from a JSON file.
    pub fn from_path(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading manifest {}: {e}", path.display()))?;
        let m: Manifest = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parsing manifest {}: {e}", path.display()))?;
        Ok(m)
    }
}

/// Resolve a dataset by manifest query (SPEC §8): scan `manifests_dir` for a
/// manifest matching tier+schema+codec, never by hard-coded path.
pub fn resolve(manifests_dir: &Path, q: &ManifestQuery) -> anyhow::Result<Manifest> {
    let mut found = Vec::new();
    if manifests_dir.is_dir() {
        for entry in std::fs::read_dir(manifests_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(m) = Manifest::from_path(&path) {
                if m.matches(q) {
                    found.push(m);
                }
            }
        }
    }
    match found.len() {
        0 => anyhow::bail!(
            "no dataset manifest matches tier={} schema={} codec={} in {}",
            q.tier,
            q.schema,
            q.codec,
            manifests_dir.display()
        ),
        1 => Ok(found.pop().unwrap()),
        n => anyhow::bail!(
            "{n} manifests match tier={} schema={} codec={}; ids are ambiguous",
            q.tier,
            q.schema,
            q.codec
        ),
    }
}

/// The on-disk directory holding the dataset's SSTable files, resolved relative
/// to the repo root from the manifest's `path_hint`.
pub fn data_dir(repo_root: &Path, m: &Manifest) -> PathBuf {
    repo_root.join(&m.path_hint)
}

/// Verify the cached binary's SHA-256 against the manifest (SPEC §8.3). Computes
/// the digest over every `*-Data.db` file in the dataset dir, in sorted order.
/// Returns the computed hex digest; the caller compares against `m.sha256`.
pub fn compute_data_sha256(dataset_dir: &Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};

    let mut data_files: Vec<PathBuf> = Vec::new();
    collect_data_files(dataset_dir, &mut data_files)?;
    data_files.sort();
    if data_files.is_empty() {
        anyhow::bail!("no *-Data.db files found under {}", dataset_dir.display());
    }

    let mut hasher = Sha256::new();
    for f in &data_files {
        let bytes = std::fs::read(f)?;
        hasher.update(&bytes);
    }
    Ok(hex(&hasher.finalize()))
}

fn collect_data_files(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_data_files(&path, out)?;
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("-Data.db"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
