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

## Test roadmap & status

Validation follows a maturity progression — **Unknown → Known → Improved →
Regression-locked** — tracked in **[`TEST_PLAN.md`](TEST_PLAN.md)** (correctness
matrix, per-version runbook, progress dashboard, engine ledger).

| Phase | What | Status |
|---|---|---|
| 1 · Unknown → Known | characterize correctness, perf, memory, bottlenecks | 🟡 in progress (3/4) |
| 2 · Improvement | file bottlenecks upstream, recalibrate targets from data, validate fixes | 🟡 in progress (1/3) |
| 3 · Regression-locked | asserted correctness gate + baseline perf gates + CI | ⬜ not started |

**Delivered:** read suite (full_scan / point_lookup / clustering_slice /
type_heavy / wide_partition), write suite (ingest WAL-on/off, flush, compaction),
mixed suite (read_while_write, open_loop with coordinated-omission correction),
Cassandra corpus generator + manifests, codec sweep, Markdown report + goals
scorecard, and `dhat` heap profiling. See `TEST_PLAN.md` for what's open and
[the issues](https://github.com/pmcfadin/cqlite-perf/issues) for sequencing.

**Engine baselines:** validated against **v0.11.0** and **main@9054734**
(post-#788/#790); re-pin to **v0.12.0** pending its tag (#16).
