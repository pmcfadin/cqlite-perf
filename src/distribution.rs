//! Key distributions for read workloads (SPEC §6): Zipfian (default) and
//! uniform, both seeded for reproducibility.
//!
//! M0 only needs the type to exist; write.ingest uses monotonic keys. The
//! generators are wired into the read suite in M1.

use rand::rngs::StdRng;
use rand::SeedableRng;
use rand_distr::{Distribution, Zipf};

/// A seeded key generator over `[0, n)`.
pub enum KeyGen {
    Uniform { n: u64, rng: StdRng },
    Zipfian { zipf: Zipf<f64>, rng: StdRng },
}

impl KeyGen {
    pub fn new(kind: &str, n: u64, seed: u64) -> anyhow::Result<Self> {
        let rng = StdRng::seed_from_u64(seed);
        match kind {
            "uniform" => Ok(KeyGen::Uniform { n, rng }),
            "zipfian" => {
                // Exponent 1.0 is the canonical Zipf skew for cache-tail
                // workloads; revisit per-suite in M1 if needed.
                let zipf = Zipf::new(n, 1.0)
                    .map_err(|e| anyhow::anyhow!("invalid Zipf parameters: {e}"))?;
                Ok(KeyGen::Zipfian { zipf, rng })
            }
            other => anyhow::bail!("unknown distribution '{other}'"),
        }
    }

    /// Draw the next key index in `[0, n)`.
    pub fn next_key(&mut self) -> u64 {
        match self {
            KeyGen::Uniform { n, rng } => {
                use rand::Rng;
                rng.gen_range(0..*n)
            }
            // Zipf yields values in [1, n]; shift to [0, n).
            KeyGen::Zipfian { zipf, rng } => (zipf.sample(rng) as u64).saturating_sub(1),
        }
    }
}
