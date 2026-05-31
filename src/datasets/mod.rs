//! Dataset manifest model (SPEC §8.3) and selection-by-query (SPEC §8).
//!
//! Binaries are never committed — only manifests + checksums. The harness
//! selects datasets by manifest query (tier + schema + codec), never by
//! hard-coded path, and verifies the cached binary's SHA-256 on run start.
//!
//! M0 defines the manifest type. The Cassandra generator (cassandra_gen.rs)
//! and writer generator (writer_gen.rs) land in M1/M2.

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
}
