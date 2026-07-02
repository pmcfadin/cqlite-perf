#!/usr/bin/env bash
# One-command perf run → HTML report with charts. Run it whenever.
#
#   scripts/run-perf-report.sh [MODE] [OUT_DIR]
#
#   MODE:
#     quick     read sanity: full_scan + point_lookup + clustering_slice,
#               1 trial (~1 min). Good for "did that engine bump do anything?"
#     read      full read suite, 3 trials (configs/read-suite.toml)  [default]
#     headline  read suite with the 5-codec sweep (configs/read-headline.toml)
#     full      read suite + the whole write+mixed suite (~5 min)
#
# Every mode: build → datasets --check → `validate` correctness gate (hard —
# perf numbers on wrong rows are worthless) → run → SUMMARY/SCORECARD →
# REPORT.html with SVG charts (this run + cross-version trend from reports/*).
# All runs of a day append to the same reports/<date>-<version>/results.jsonl,
# so quick and full modes accumulate into one report dir.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

MODE="${1:-read}"
OUT="${2:-reports}"
BIN=./target/release/cqlite-perf

case "$MODE" in quick|read|headline|full) ;; *)
  echo "usage: $0 [quick|read|headline|full] [OUT_DIR]" >&2; exit 2 ;;
esac

echo "▶ build"
cargo build --release --quiet

echo "▶ corpus content check"
"$BIN" datasets --check   # hard-fails only on a SHA mismatch; missing corpora skip

echo "▶ correctness gate (validate)"
"$BIN" validate

echo "▶ run: $MODE"
case "$MODE" in
  quick)
    for W in read.full_scan read.point_lookup read.clustering_slice; do
      "$BIN" run --workload "$W" --duration 5s --warmup 2s --trials 1 --out "$OUT"
    done ;;
  read)
    "$BIN" run --config configs/read-suite.toml --out "$OUT" ;;
  headline)
    "$BIN" run --config configs/read-headline.toml --out "$OUT" ;;
  full)
    "$BIN" run --config configs/read-suite.toml --out "$OUT"
    scripts/run-write-mixed.sh "$OUT" ;;
esac

DIR=$(ls -d "$OUT"/*/ | tail -1)
RESULTS="${DIR}results.jsonl"

echo "▶ report"
"$BIN" report --results "$RESULTS" --goals goals.toml
python3 scripts/render_report.py --results "$RESULTS" --history reports

REPORT="${DIR}REPORT.html"
echo "✓ done: $REPORT"
[[ "$(uname)" == "Darwin" && -z "${CI:-}" ]] && open "$REPORT" || true
