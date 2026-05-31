# cqlite-perf — Repository Specification

**Date:** 2026-05-29
**Status:** Spec — ready to bootstrap as a standalone repo
**Companion:** `2026-05-29-performance-testing-design.md` (the design this implements)

This is the build spec for `pmcfadin/cqlite-perf`, the external macro-benchmark
harness for CQLite. It is self-contained: copy this file into the new repo as
`SPEC.md` and work from it. It describes the read, write, and mixed performance
suites, dataset generation, metrics, reporting, CI, and cross-version
benchmarking.

The in-repo Criterion micro-benchmarks (regression gating, Phase 1) live in
the main `cqlite` repo under `cqlite-core/benches/` and are **out of scope**
here. This repo owns Phase 2 (headline numbers) and Phase 3 (bottleneck
hunting).

---

## 1. Purpose

Produce trustworthy, reproducible answers to:

- **Throughput** — reads/sec, writes/sec, rows/sec, MB/sec.
- **Latency** — p50/p90/p95/p99/p99.9 + max, tails preserved.
- **Volume** — how the above scale as data grows from ~100 MB to 10 GB+.

across **read**, **write**, and **mixed** workloads, over a sweep of
concurrency levels, key distributions, dataset tiers, and compression codecs.

The harness must support **cross-version benchmarking** — running the same
suite against two pinned cqlite versions to quantify release-over-release
change. This is the reason it is a separate repo rather than an in-tree crate.

---

## 2. Relationship to cqlite

cqlite is **not published to crates.io**. This repo depends on it via **git
tag**, which is what enables version pinning and comparison.

```toml
# Cargo.toml — Rust harness dependency
[dependencies]
cqlite-core = { git = "https://github.com/pmcfadin/cqlite", tag = "v0.9.2" }
```

- To benchmark a different version, change the tag (or use a Cargo `[patch]`
  / feature to build two versions side by side — see §15).
- For the bindings harness (Phase 2), install the matching published Python
  wheel / npm package, or build from the same pinned git ref, so the
  binding numbers track the same cqlite version as the Rust numbers.
- This repo is **not** a member of the cqlite Cargo workspace. It is an
  independent project with its own `Cargo.lock` committed.

### cqlite public API this harness drives

Read path (`cqlite_core::Database`, all async):

```rust
Database::open(path: &Path, config: Config) -> Result<Database>
Database::open_with_discovered_sstables(...) -> Result<Database>
db.execute(query: &str) -> Result<QueryResult>
db.execute_streaming(query: &str, ...) -> Result<...>   // memory-bounded
db.prepare(query: &str) -> Result<Arc<PreparedQuery>>
db.execute_prepared(stmt, &[Value]) -> Result<QueryResult>
db.stats() -> Result<DatabaseStats>
db.flush() / db.compact() / db.shutdown()
```

Write path (`cqlite_core::storage::write_engine::WriteEngine`):

```rust
WriteEngineConfig::new(data_dir, wal_dir, schema)
    .with_flush_threshold(n)
    .with_hard_limit(n)
WriteEngine::new(config) -> Result<WriteEngine>
we.write(mutation) / we.write_async(mutation) -> Result<()>
we.execute(cql: &str) -> Result<()>          // CQL string → mutation
we.flush() -> Result<Option<SSTableInfo>>
we.memtable_size() / we.memtable_row_count() / we.wal_size() / we.generation()
we.maintenance_step(budget: Duration) -> Result<MaintenanceReport>  // compaction
we.maintenance_stats() -> CompactionStats
```

> Verify these signatures against the pinned tag before relying on them; the
> write engine is M5-era and still evolving. If a needed entry point is not
> `pub`, file an issue upstream rather than reaching into internals.

---

## 3. Repository Layout

```
cqlite-perf/
├── Cargo.toml                  # standalone project, Cargo.lock committed
├── Cargo.lock
├── SPEC.md                     # this file
├── README.md                   # quickstart + latest headline table
├── rust-toolchain.toml         # pin toolchain to match cqlite (1.85+)
├── .gitignore                  # ignores datasets/**, target/, reports/*.local
│
├── src/
│   ├── main.rs                 # CLI entry (cqlite-perf run …)
│   ├── config.rs               # run config (TOML) parsing
│   ├── runner.rs               # the measurement loop (warmup, duration, repeat)
│   ├── metrics.rs              # HDR histograms, metric envelope, aggregation
│   ├── report.rs               # JSON + Markdown emitters
│   ├── distribution.rs         # Zipfian / uniform key generators
│   ├── workloads/
│   │   ├── mod.rs              # Workload trait
│   │   ├── read.rs             # point/slice/scan/wide/type-heavy
│   │   ├── write.rs            # ingest/flush/compaction
│   │   └── mixed.rs            # read-while-write, open-loop target rate
│   └── datasets/
│       ├── mod.rs              # manifest model + selection-by-query
│       ├── cassandra_gen.rs    # Docker Cassandra → SSTable corpus
│       └── writer_gen.rs       # cqlite write engine → SSTable corpus
│
├── bindings-perf/              # Phase 2 — driven from outside Rust
│   ├── python/                 # pytest-benchmark or custom timing harness
│   └── node/                   # tinybench or custom timing harness
│
├── datasets/                   # GITIGNORED binaries; manifests committed
│   └── manifests/
│       ├── read-basic-S-lz4.json
│       ├── read-wide-L-zstd.json
│       └── …
│
├── schemas/                    # scaled-up copies of cqlite test schemas
│   ├── basic.cql
│   ├── collections.cql
│   ├── timeseries.cql
│   └── wide_rows.cql
│
├── configs/                    # named run configs (TOML)
│   ├── regression.toml         # quick smoke (S tier, low concurrency)
│   ├── headline.toml           # full S/M/L × concurrency × codec sweep
│   └── soak.toml               # long mixed run
│
├── reports/                    # committed headline outputs (JSON + MD)
│   └── 2026-XX-XX-vX.Y.Z/
│
├── scripts/
│   ├── gen-corpus.sh           # orchestrates Cassandra Docker generation
│   ├── drop-page-cache.sh      # cold-cache helper (per-OS)
│   └── compare-versions.sh     # cross-version run + diff
│
└── .github/workflows/
    ├── smoke.yml               # PR-gate: S tier, 1 trial, sanity only
    └── headline.yml            # scheduled/manual: full sweep, archives reports
```

---

## 4. Cargo Setup

```toml
# Cargo.toml
[package]
name = "cqlite-perf"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
cqlite-core = { git = "https://github.com/pmcfadin/cqlite", tag = "v0.9.2" }
tokio = { version = "1", features = ["full"] }
hdrhistogram = "7"               # latency percentiles, coordinated-omission aware
rand = "0.8"
rand_distr = "0.4"               # Zipfian distribution
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
clap = { version = "4", features = ["derive"] }
anyhow = "1"
sysinfo = "0.30"                 # RSS / CPU sampling
sha2 = "0.10"                    # dataset checksums

[dev-dependencies]
criterion = "0.5"                # optional shared bench helpers

[profile.release]
debug = true                     # keep symbols for flamegraphs (Phase 3)
lto = "thin"
codegen-units = 1
```

`rust-toolchain.toml` pins the same toolchain cqlite builds with (1.85+) to
avoid codegen-difference noise in numbers.

---

## 5. Core Abstractions

### `Workload` trait

Every suite implements one trait so the runner, metrics, and reporting stay
uniform.

```rust
#[async_trait::async_trait]
pub trait Workload {
    /// Stable identifier, e.g. "read.point_lookup".
    fn name(&self) -> &'static str;

    /// One-time setup: open DB / build write engine / preselect keys.
    async fn setup(&mut self, ctx: &RunContext) -> anyhow::Result<()>;

    /// Execute exactly one operation. Timed by the runner.
    /// Returns rows touched (for rows/sec accounting).
    async fn op(&self, worker: usize) -> anyhow::Result<u64>;

    /// Teardown: shutdown DB, drop temp dirs.
    async fn teardown(&mut self) -> anyhow::Result<()>;
}
```

### `RunContext`

Immutable per-run config handed to every workload: dataset manifest, tier,
concurrency, distribution, codec, warmup/duration/trials, seed, cold-cache
flag.

### `Runner`

Owns the measurement loop. For each (workload × concurrency × trial):

1. `setup()`.
2. Optional cold-cache: call `scripts/drop-page-cache.sh`.
3. Warmup: run `op()` for `warmup_secs`, results discarded.
4. Measure: spawn `concurrency` workers, each looping `op()` for
   `duration_secs`; record each op's latency into a per-worker HDR histogram;
   merge histograms at the end.
5. Sample RSS/CPU via `sysinfo` on a background ticker.
6. `teardown()`.
7. Emit a `RunResult` (the metric envelope, §7).

Closed-loop by default (workers issue the next op as soon as the prior
returns). Open-loop variant (§6, mixed) drives at a target rate with
coordinated-omission correction.

---

## 6. Workload Catalog

All workloads are parameterized by tier, concurrency, distribution, codec.

### Read suite (`src/workloads/read.rs`)

| Name | Operation | What it stresses |
|------|-----------|------------------|
| `read.point_lookup` | `SELECT … WHERE pk = ?` (single partition) | Index.db/Summary.db + partition seek; latency-sensitive |
| `read.clustering_slice` | range over clustering keys in one partition | in-partition scan |
| `read.full_scan` | `SELECT *` streaming all rows | raw decode throughput → rows/sec headline |
| `read.wide_partition` | full scan over `wide_rows` shape | large-partition behavior |
| `read.type_heavy` | scan over collections/UDT shape | deserialization cost vs. I/O |

Keys for point/slice are drawn from the configured distribution (default
Zipfian via `rand_distr::Zipf`, uniform alternative) using the fixed seed, so
runs are reproducible.

### Write suite (`src/workloads/write.rs`)

Built on `WriteEngine`. The write suite doubles as the volume generator
(§8.2).

| Name | Operation | Metric |
|------|-----------|--------|
| `write.ingest` | `we.write(mutation)` sustained | writes/sec; run WAL-on and WAL-off variants |
| `write.flush` | fill memtable to threshold, time `we.flush()` | flush MB/sec, flush latency |
| `write.compaction` | create K SSTables, `we.maintenance_step(budget)` to merge | compaction wall-time, read-amp, output size |

### Mixed suite (`src/workloads/mixed.rs`)

| Name | Operation | Metric |
|------|-----------|--------|
| `mixed.read_while_write` | N reader workers over existing SSTables + M writer workers ingesting | read p99 **under** write load |
| `mixed.open_loop` | writers driven at fixed target rate; readers closed-loop | does read p99 hold as write rate climbs? |

`mixed.open_loop` uses HDR coordinated-omission correction: latency is measured
against the *intended* issue time, not the actual start, so a backed-up system
cannot hide its stalls.

---

## 7. Metric Envelope

Every `RunResult` carries the same fields, serialized to JSON.

```jsonc
{
  "workload": "read.point_lookup",
  "cqlite_version": "v0.9.2",
  "cqlite_git_sha": "ff3737e",
  "harness_version": "0.1.0",
  "dataset": { "tier": "M", "rows": 25000000, "bytes": 3221225472,
               "codec": "lz4", "schema": "basic" },
  "distribution": "zipfian",
  "concurrency": 8,
  "cache": "warm",                 // or "cold"
  "throughput": { "ops_per_sec": 142000.0, "rows_per_sec": 142000.0,
                  "mb_per_sec": null },
  "latency_us": { "p50": 48, "p90": 71, "p95": 88, "p99": 140,
                  "p999": 410, "max": 2200 },
  "resource": { "peak_rss_bytes": 96468992, "cpu_pct_mean": 612.0,
                "alloc_bytes": null, "alloc_count": null },
  "trials": 3,
  "variance": { "ops_per_sec_cv": 0.03 },
  "host": { "cpu": "Apple M2 Pro", "cores": 12, "ram_gb": 32,
            "os": "darwin 25.5.0" },
  "duration_secs": 30,
  "warmup_secs": 5,
  "seed": 42
}
```

`alloc_*` populated only in Phase 3 profiling runs (via `dhat`). `peak_rss`
always populated and checked against the <128 MB target where the workload is
expected to honor it (streaming reads); full materializing scans may exceed it
by design — flag, don't fail.

---

## 8. Dataset Generation

Two generators by role. Binaries are **never committed** — only manifests and
checksums. This mirrors cqlite's own `test-data/scripts/fetch-datasets.sh`
split.

### 8.1 Cassandra-generated (canonical read corpus)

`scripts/gen-corpus.sh` + `src/datasets/cassandra_gen.rs`:

1. Start Cassandra 5.0 in Docker.
2. Create keyspace/table from `schemas/*.cql`, setting compression per codec
   variant:
   - `LZ4Compressor`, `SnappyCompressor`, `DeflateCompressor`,
     `ZstdCompressor`, or `compression = {}` (none).
3. Load the tier's row count (CQL writer or `cassandra-stress`).
4. `nodetool flush`.
5. Copy out the SSTable directory.
6. Write a `manifest.json` (§8.3); compute SHA-256 over the Data.db files.

These authentic files back the **read suite** and the codec sweep.

### 8.2 CQLite-write-engine-generated (write / volume)

`src/datasets/writer_gen.rs`: drive `WriteEngine` to produce SSTables. The
**write suite is this generator** — benchmarking the write path and producing
an L-tier dataset are the same run. Fast, no Docker. Used for write/volume
work; not used to validate read *correctness* (that would be circular), only
read *performance shape* when an authentic Cassandra corpus is unavailable.

### 8.3 Manifest model

```json
{
  "id": "read-wide-L-zstd",
  "tier": "L",
  "schema": "wide_rows",
  "codec": "zstd",
  "rows": 100000000,
  "bytes": 12884901888,
  "generator": "cassandra-5.0",
  "generator_version": "5.0.0",
  "cqlite_schema_ref": "schemas/wide_rows.cql",
  "sha256": "…",
  "created_utc": "2026-06-01T00:00:00Z",
  "path_hint": "datasets/read-wide-L-zstd/"
}
```

The harness selects datasets by **manifest query** (tier + schema + codec),
never by hard-coded path. On run start it checks the cached binary's SHA-256
against the manifest; mismatch or absence triggers regeneration (or an error
under `--no-regen`).

### 8.4 Tiers

| Tier | Size | Rows |
|------|------|------|
| S | ~100 MB | ~1M |
| M | 1–5 GB | 10–50M |
| L | 10 GB+ | 100M+, wide partitions |

S-tier may be cached in CI artifact storage. M/L are generated on demand on
the perf host.

### 8.5 `.gitignore`

```
/target
/datasets/**
!/datasets/manifests/
!/datasets/manifests/**
/reports/*.local.*
```

---

## 9. Methodology Guardrails

Non-negotiable for trustworthy numbers:

- **Warmup** discarded (settle page cache + lazy init).
- **Cold-cache variant**: `scripts/drop-page-cache.sh` before the run; report
  `cache: "cold"` separately from warm. SSTable reads are I/O-bound — cold vs.
  warm is a real, reportable distinction, not noise.
- **Duration-based** measurement (fixed wall-time, count ops), not fixed-count.
- **Coordinated-omission correction** on all open-loop runs.
- **≥3 trials**, report median + coefficient of variation; never a single run.
- **Quiet machine**: headline runs on a dedicated host, optional CPU pinning
  (`taskset` on Linux), no background load. Document the protocol in README.

---

## 10. Compression-Codec Sweep

Codec is a dataset-generation axis (§8.1), surfaced in every read result via
`dataset.codec`. The headline report cross-tabulates scan throughput and
on-disk size by codec, e.g.:

| Codec | Size (M tier) | Scan rows/sec | p99 µs |
|-------|---------------|---------------|--------|
| none | … | … | … |
| lz4 | … | … | … |
| snappy | … | … | … |
| deflate | … | … | … |
| zstd | … | … | … |

The deliverable claim is the size-vs-speed tradeoff curve.

---

## 11. Output & Reporting

- **JSON** (`report.rs`): one `RunResult` object per (workload × concurrency ×
  cache × codec × tier). Appended to a run-scoped JSONL file under
  `reports/<date>-<version>/results.jsonl`. This is the canonical record and
  the input to regression diffing and cross-version comparison.
- **Markdown** (`report.rs`): auto-generated `reports/<date>-<version>/
  SUMMARY.md` with the headline tables (throughput-by-concurrency,
  latency-by-tier, codec sweep, version comparison). The README's "latest
  numbers" section links here.
- **Criterion HTML**: only if shared Criterion helpers are used for any
  micro-measurement; not the primary path.

---

## 12. Harness CLI

```bash
# Run a named config
cqlite-perf run --config configs/headline.toml

# Run one workload ad hoc
cqlite-perf run \
  --workload read.point_lookup \
  --tier M --codec lz4 --distribution zipfian \
  --concurrency 1,2,4,8,16 \
  --warmup 5s --duration 30s --trials 3 \
  --cold-cache \
  --out reports/

# Generate a dataset (Cassandra)
cqlite-perf gen --source cassandra --tier M --codec zstd --schema basic

# Generate a dataset (write engine) — also a write benchmark
cqlite-perf gen --source writer --tier L --schema wide_rows

# Cross-version comparison
cqlite-perf compare --base v0.9.2 --candidate v0.10.0 \
  --config configs/headline.toml

# List/validate cached datasets against manifests
cqlite-perf datasets --check
```

`run` config file (TOML) example (`configs/headline.toml`):

```toml
warmup = "5s"
duration = "30s"
trials = 3
concurrency = [1, 2, 4, 8, 16]
distributions = ["zipfian", "uniform"]
codecs = ["none", "lz4", "snappy", "deflate", "zstd"]
tiers = ["S", "M", "L"]
cold_cache = true
seed = 42

[[suite]]
name = "read"
workloads = ["point_lookup", "clustering_slice", "full_scan",
             "wide_partition", "type_heavy"]

[[suite]]
name = "write"
workloads = ["ingest", "flush", "compaction"]

[[suite]]
name = "mixed"
workloads = ["read_while_write", "open_loop"]
```

---

## 13. Bindings Harness (Phase 2)

Under `bindings-perf/`, driven from outside Rust so numbers reflect real FFI +
language overhead.

- **Python** (`bindings-perf/python/`): install the cqlite wheel matching the
  pinned tag (`maturin build --release` from the cqlite repo at that tag, or
  the published PyPI wheel). Time `db.execute()` / `db.execute_streaming()`
  with a custom loop emitting the same metric envelope as the Rust harness.
  Note the documented concurrent-query warm-up caveat (CLAUDE.md) — issue one
  warm-up query before parallel access. Account for the GIL: prefer
  multiprocess for true concurrency sweeps.
- **Node** (`bindings-perf/node/`): install `@cqlite/node` matching the tag.
  Use `executeNative()` (not the deprecated `execute()`), time with
  `tinybench` or a custom loop, emit the same envelope.

Goal: a "native Rust does X, Python does ~0.7X, Node does ~0.8X" overhead
table. CLI is a thin add-on — measure `cqlite --query …` one-shot to capture
process-startup + serialization overhead, not as a primary throughput surface.

---

## 14. CI

- **`smoke.yml`** (on PR to this repo): build, then `cqlite-perf run --config
  configs/regression.toml` — S tier, 1 trial, low concurrency. Sanity only,
  fast. Asserts the harness runs and emits valid JSON; does **not** gate on
  absolute numbers.
- **`headline.yml`** (manual `workflow_dispatch` + scheduled): full sweep on
  the dedicated perf host (self-hosted runner). Archives
  `reports/<date>-<version>/` as artifacts and optionally commits the SUMMARY.

The main cqlite repo's PRs are **never** gated on this repo. Regression gating
for PRs is the in-repo Criterion job (Phase 1), separate from here.

---

## 15. Cross-Version Benchmarking

The payoff of being a separate repo.

`scripts/compare-versions.sh` / `cqlite-perf compare`:

1. Build the harness against `--base` tag (e.g. via a Cargo feature selecting a
   `[patch]`, or two checkouts of this repo each pinning a different tag, or a
   git-dep override). Simplest robust approach: **two target dirs, two pinned
   `Cargo.toml`s**, run each, then diff JSONL.
2. Run the identical config against both, same datasets (selected by manifest —
   identical bytes guaranteed by checksum).
3. `report.rs` emits a delta table: per workload, percent change in
   throughput and p99, flagged red past a threshold (e.g. >10% p99
   regression).

Document the chosen mechanism in README once implemented; do not over-engineer
it before the first comparison is actually needed.

---

## 16. Build Order (Milestones)

1. **M0 — Skeleton**: Cargo project, `Workload` trait, `Runner` with
   warmup/duration/trials, HDR metrics, JSON emitter. One trivial workload
   (`read.full_scan`) against a hand-placed S-tier dataset. Proves the loop.
2. **M1 — Read suite + datasets**: all read workloads; Cassandra generator +
   manifests; distribution knob; codec sweep; Markdown report.
3. **M2 — Write + mixed suites**: WriteEngine-backed workloads; writer
   generator; open-loop + coordinated omission.
4. **M3 — Bindings harness**: Python + Node drivers; overhead table.
5. **M4 — CI + cross-version**: smoke/headline workflows; `compare`.
6. **M5 — Profiling (Phase 3)**: `dhat` allocation tracking, flamegraph
   scripts, <128 MB assertions where applicable.

Each milestone is independently useful and produces real numbers.

---

## 17. Open Items

- Choose the perf host (dedicated Mac/Linux box vs. cloud) and write the
  "quiet machine" protocol.
- Decide trend storage for regression history (committed JSONL vs. external
  store) and the regression threshold.
- Confirm `cassandra-stress` schema mapping to the cqlite test schemas, or
  write a small CQL loader instead.
- Pick the concrete cross-version build mechanism (§15) at M4, not before.
- Confirm which write-engine entry points are `pub` at the pinned tag; file
  upstream issues for any gaps.

---

## 18. Bootstrap Checklist

```bash
# 1. Create and init
mkdir cqlite-perf && cd cqlite-perf
git init
cargo init --name cqlite-perf

# 2. Drop in this file as SPEC.md, plus Cargo.toml (§4), .gitignore (§8.5),
#    rust-toolchain.toml, and the directory skeleton (§3).

# 3. Pin the cqlite dependency to the current release tag
#    cqlite-core = { git = "…/cqlite", tag = "v0.9.2" }

# 4. Copy scaled-up schemas from cqlite/test-data/schemas/ into schemas/

# 5. First commit + push
git add -A && git commit -m "chore: bootstrap cqlite-perf harness (M0 skeleton)"
gh repo create pmcfadin/cqlite-perf --private --source=. --push

# 6. Implement M0 (Runner + one workload), confirm it emits valid JSON,
#    then iterate through milestones M1→M5.
```
