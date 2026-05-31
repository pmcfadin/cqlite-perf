# cqlite-perf

External macro-benchmark harness for [CQLite](https://github.com/pmcfadin/cqlite) —
the read, write, and mixed performance suites (Phase 2 headline numbers, Phase 3
bottleneck hunting). The in-repo Criterion micro-benchmarks live in the main
cqlite repo and are out of scope here.

See [`SPEC.md`](SPEC.md) for the full design.

## Standalone by git tag

This repo is **not** part of the cqlite Cargo workspace. It depends on cqlite via
a pinned **git tag**, which is what enables version pinning and cross-version
comparison:

```toml
cqlite-core = { git = "https://github.com/pmcfadin/cqlite.git", tag = "v0.9.1", features = ["write-support"] }
```

No local paths, no vendored binaries. `Cargo.lock` is committed for reproducible
builds. Bump the tag to benchmark a different cqlite release.

The toolchain is pinned to **Rust 1.88.0** in `rust-toolchain.toml`, matching the
toolchain cqlite builds with, to avoid codegen-difference noise in the numbers.

## Quickstart

```bash
# Smoke run (S tier, low concurrency) — the sanity config
cargo run --release -- run --config configs/regression.toml

# One workload ad hoc
cargo run --release -- run \
  --workload write.ingest \
  --concurrency 1,2,4,8 \
  --warmup 5s --duration 30s --trials 3 \
  --out reports/
```

Results are appended as JSONL to `reports/<date>-<harness-version>/results.jsonl`
— the canonical record and the input to regression diffing and cross-version
comparison (SPEC §11).

## Status — M0 (skeleton)

The measurement loop is proven end-to-end:

- `Workload` trait + `RunContext` (SPEC §5)
- `Runner`: warmup → duration-based measurement → trials → closed-loop workers,
  with best-effort RSS/CPU sampling (SPEC §5)
- HDR latency histograms + the full metric envelope (SPEC §7)
- JSON/JSONL emitter (SPEC §11)
- One real workload — **`write.ingest`** — driving cqlite's `WriteEngine` with
  zero external setup (no Docker, no pre-built corpus), which is why it, rather
  than `read.full_scan`, is the M0 proof workload.

Subsequent milestones (SPEC §16):

- **M1** — read suite + Cassandra dataset generator + manifests + codec sweep +
  Markdown report
- **M2** — write/flush/compaction + mixed suite + open-loop coordinated-omission
- **M3** — Python/Node bindings overhead table
- **M4** — CI (smoke + headline) + cross-version `compare`
- **M5** — `dhat` allocation profiling + flamegraphs + <128 MB assertions
