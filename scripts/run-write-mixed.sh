#!/usr/bin/env bash
# Reproduce the M2 write + mixed suite into one report dir, then regenerate
# SUMMARY.md + SCORECARD.md from the full results.jsonl.
#
# The workloads need different driving (concurrency sweep for ingest; conc=1 +
# warmup 0 for the single-engine flush/compaction; conc>=3 + warmup 0 for the
# mixed cohorts), so they run as separate invocations that all APPEND to the
# same reports/<date>-<version>/results.jsonl, and a final `report` re-renders.
#
# Usage: scripts/run-write-mixed.sh [OUT_DIR]   (default OUT_DIR=reports)
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"

OUT="${1:-reports}"
BIN=./target/release/cqlite-perf

cargo build --release

# Ingest: per-worker engines → real concurrency scaling. WAL-on (fsync-bound)
# and WAL-off (CPU-bound) variants.
"$BIN" run --workload write.ingest         --concurrency 1,2,4,8 --warmup 1s --duration 6s  --trials 3 --out "$OUT"
"$BIN" run --workload write.ingest_waloff  --concurrency 1,2,4,8 --warmup 1s --duration 6s  --trials 3 --out "$OUT"

# Flush + compaction: single-engine, no warmup so every cycle counts.
"$BIN" run --workload write.flush       --warmup 0 --duration 10s --trials 3 --out "$OUT"
"$BIN" run --workload write.compaction  --warmup 0 --duration 15s --trials 3 --out "$OUT"

# Mixed: 2 writers + (conc-2) readers; open_loop drives writers at a target rate.
"$BIN" run --workload mixed.read_while_write --concurrency 8 --warmup 0 --duration 12s --trials 3 --out "$OUT"
"$BIN" run --workload mixed.open_loop        --concurrency 8 --warmup 0 --duration 12s --trials 3 --out "$OUT"

# Re-render the unified report from the full accumulated JSONL.
DIR=$(ls -d "$OUT"/*/ | tail -1)
"$BIN" report --results "${DIR}results.jsonl" --goals goals.toml
echo "✓ unified report in ${DIR}"
