# Linux Baseline — location and provenance

## Authoritative source

The authoritative Linux baseline is produced by the `linux-baseline` job in
`.github/workflows/ci.yml` and uploaded as a GitHub Actions artifact named
`linux-baseline-master` (retained 90 days) on every push to `master`.

To retrieve it: Actions tab → most recent push to master → `linux-baseline-master`
artifact → download and inspect `results.jsonl` + `SUMMARY.md`.

## Why not a committed file

Committing raw perf numbers here would drift immediately: the numbers depend on
the GitHub-hosted runner hardware, cqlite-core engine version, and corpus
generation timestamp. The artifact approach keeps the number associated with the
exact commit + engine rev that produced it, and the CI compare step in the
workflow automatically diffs against the prior run.

## What the CI run measures

Runner: `ubuntu-latest` (x86-64, GitHub-hosted, real Linux — not virtualized).
Workloads: `read.full_scan`, `read.point_lookup`.
Duration: 10s measurement + 5s warmup, 1 trial each.
Corpus: S-tier (100k rows), read-basic-S-lz4, verified against manifest SHA256.

These are the authoritative Linux throughput and latency numbers. The macOS
baselines under `reports/2026-*/` are arm64/macOS and are kept as historical
reference; they are NOT comparable to the Linux CI numbers.

## Local virtualized numbers (for reference only)

The W4-A Docker image (`ci/Dockerfile`) can produce Linux-in-container numbers
on Apple Silicon via `docker run cqlite-perf-ci run --workload read.full_scan ...`.
These numbers are from a **virtualized Linux/arm64 environment** (Docker Desktop
on macOS) and must NOT be used as the Linux performance baseline. They are useful
for correctness checks and relative comparisons (before/after a code change)
but not for absolute Linux benchmark claims.

See `ci/README.md` for the Docker run instructions.
