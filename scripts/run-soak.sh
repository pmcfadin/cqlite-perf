#!/usr/bin/env bash
# Soak profiles (issue #33, epic #30 W3): sustained-duration runs that catch
# regressions that only show up over hours (throughput decay, RSS growth,
# tail-latency drift) — not in a `run-write-mixed.sh`-style few-second run.
# Uses the runner's soak time-series mode (#31, `--snapshot-interval`), whose
# derived `custom.trend.*` metrics goals.toml gates as self-relative "soak
# trend" goals (#32).
#
# Two profiles, `soak.mixed` and `soak.ingest` (documented for humans in
# configs/soak.toml, see that file's header for why it is NOT a runnable
# `--config`): RunConfig (src/config.rs) applies one shared
# warmup/duration/trials/concurrency across every `[[suite]]` workload, and
# these two profiles need genuinely different concurrency/warmup/workload
# values. scripts/run-write-mixed.sh already set this precedent for the same
# reason (write.ingest / write.flush / write.compaction / mixed.* each need
# different --concurrency/--duration/--warmup): it calls `cqlite-perf run
# --workload <name> <flags>` directly, once per workload, all appending to one
# timestamped report dir, then a final `report` re-render. This script follows
# that same pattern.
#
# Usage: scripts/run-soak.sh [--profile mixed|ingest|all] [--duration 2h] [OUT_DIR]
#   --profile   which soak profile(s) to run (default: all)
#   --duration  overrides BOTH profiles' --duration (default: 2h) so the same
#               script serves a fast smoke test (--duration 3m) and the slow
#               authoritative run (--duration 4h+, TEST_PLAN.md Reference E).
#   OUT_DIR     positional, default "reports"
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"

PROFILE="all"
DURATION="2h"
OUT="reports"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="$2"; shift 2 ;;
    --duration)
      DURATION="$2"; shift 2 ;;
    --help|-h)
      grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)
      OUT="$1"; shift ;;
  esac
done

case "$PROFILE" in
  mixed|ingest|all) ;;
  *)
    echo "error: --profile must be mixed|ingest|all (got: '$PROFILE')" >&2
    exit 1 ;;
esac

BIN=./target/release/cqlite-perf

cargo build --release

# ── Disk-volume safety note (soak.ingest) ───────────────────────────────────
# write.ingest is WAL-on (fsync-per-write). Measured on 2026-06-26 (v0.12.0,
# reports/2026-06-26-0.1.0/results.jsonl): ~283 ops/sec at concurrency=4 (4
# independent per-worker engines, 1 row/op). The "basic" schema's on-disk
# (lz4) size is ~58.65 bytes/row (datasets/manifests/read-basic-S-lz4.json:
# 5,865,472 bytes / 100,000 rows). At the default 2h duration that is:
#   283 rows/s * 7200 s ~= 2.04M rows  ->  ~2.04M * 58.65 B ~= 120 MB on disk
# — comfortably inside a GH-hosted runner's ~14 GB free budget, even padded 5x
# for WAL overhead + per-worker directory duplication (~600 MB). A 4h+
# authoritative run doubles that to ~240 MB (~1.2 GB padded). No rate cap or
# bounded key-space overwrite mode is needed at current throughput; recompute
# before extending well past 4h or after a throughput-improving engine change
# (full math + assumptions: configs/soak.toml).
#
# KNOWN GAP: write.ingest (src/workloads/write.rs) only flushes proactively at
# the memtable threshold — it never calls `maintenance_step()`, so no
# compaction runs during this profile today (the issue asks for "periodic
# maintenance_step() cycling, reuse the write.compaction hop"). That is a
# workload/runner code change, out of scope here (config/script only) —
# tracked as a follow-up. soak.ingest as shipped measures flush-only ingest
# throughput retention + RSS slope; it does not yet exercise the
# compaction-debt scenario upstream pmcfadin/cqlite#905 is about.

run_mixed() {
  echo "=== soak.mixed (mixed.read_while_write) — duration=${DURATION} ==="
  "$BIN" run --workload mixed.read_while_write \
    --concurrency 8 --warmup 2m --duration "$DURATION" --trials 1 \
    --snapshot-interval 60s --out "$OUT"
}

run_ingest() {
  echo "=== soak.ingest (write.ingest) — duration=${DURATION} ==="
  "$BIN" run --workload write.ingest \
    --concurrency 4 --duration "$DURATION" --trials 1 \
    --snapshot-interval 60s --out "$OUT"
}

case "$PROFILE" in
  mixed) run_mixed ;;
  ingest) run_ingest ;;
  all) run_mixed; run_ingest ;;
esac

# Re-render the unified report from the full accumulated JSONL.
DIR=$(ls -d "$OUT"/*/ | tail -1)
JSONL="${DIR}results.jsonl"
"$BIN" report --results "$JSONL" --goals goals.toml
echo "✓ soak report in ${DIR}"

# ── Trend-metric one-liner summary ──────────────────────────────────────────
# Human-readable verdict without opening SCORECARD.md/results.jsonl: every
# `custom.trend.*` key on each result line, one line per soak workload run.
python3 -c "
import json

path = '$JSONL'
with open(path) as f:
    lines = [l.strip() for l in f if l.strip()]

any_trend = False
for line in lines:
    r = json.loads(line)
    custom = r.get('custom') or {}
    trend = {k: v for k, v in sorted(custom.items()) if k.startswith('trend.')}
    if not trend:
        continue
    any_trend = True
    wl = r.get('workload', '?')
    conc = r.get('concurrency', '?')
    parts = [f'{k.removeprefix(\"trend.\")}={v:.3f}' for k, v in trend.items()]
    print(f'  {wl} (concurrency={conc}): ' + '  '.join(parts))

if not any_trend:
    print('  (no custom.trend.* keys found — was --snapshot-interval set?)')
"
