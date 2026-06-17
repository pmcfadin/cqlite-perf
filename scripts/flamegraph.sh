#!/usr/bin/env bash
# scripts/flamegraph.sh <workload>
#
# Profile a single cqlite-perf read workload and emit an SVG flamegraph under
# profiles/<date>-<workload>-flamegraph.svg.
#
# OS detection:
#   Linux  — uses perf (cargo-flamegraph's perf backend, available in CI without sudo)
#   macOS  — uses dtrace (cargo-flamegraph's dtrace backend, requires sudo)
#
# The script exits with a clear message if the required profiler or privilege is
# unavailable. When capture is blocked (e.g. macOS without sudo), the script still
# succeeds with exit code 0 so CI can run the build/check steps without the profiler.
#
# Usage:
#   scripts/flamegraph.sh read.full_scan
#   scripts/flamegraph.sh read.wide_partition
#   scripts/flamegraph.sh read.type_heavy
#
# The workload runs with --warmup 0 --duration 8s --trials 1 --concurrency 1
# so a flamegraph captures ~8s of steady-state decode work, not startup overhead.

set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"

# ---------------------------------------------------------------------------
# Args
# ---------------------------------------------------------------------------
if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <workload>" >&2
    echo "  e.g. $0 read.full_scan" >&2
    exit 1
fi

WORKLOAD="$1"
DATE=$(date +%Y-%m-%d 2>/dev/null || echo "undated")
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILES_DIR="$REPO_ROOT/profiles"
OUT_SVG="$PROFILES_DIR/${DATE}-${WORKLOAD}-flamegraph.svg"

mkdir -p "$PROFILES_DIR"

# ---------------------------------------------------------------------------
# Build release binary (symbols already enabled in [profile.release])
# ---------------------------------------------------------------------------
echo "==> Building release binary (debug=true, lto=thin, codegen-units=1)..."
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"
BIN="$REPO_ROOT/target/release/cqlite-perf"

# ---------------------------------------------------------------------------
# OS + profiler detection
# ---------------------------------------------------------------------------
OS="$(uname -s)"

if [[ "$OS" == "Linux" ]]; then
    # Linux: use perf (cargo-flamegraph's default backend).
    if ! command -v flamegraph &>/dev/null && ! cargo flamegraph --version &>/dev/null 2>&1; then
        echo "==> cargo-flamegraph not found — installing..."
        cargo install flamegraph --locked
    fi
    if ! command -v perf &>/dev/null; then
        echo "ERROR: 'perf' not found. Install linux-perf or linux-tools-generic." >&2
        echo "  On Debian/Ubuntu: sudo apt-get install linux-perf" >&2
        exit 1
    fi
    echo "==> Linux: profiling via perf"
    FLAMEGRAPH_BACKEND=""  # perf is the default for cargo-flamegraph on Linux

elif [[ "$OS" == "Darwin" ]]; then
    # macOS: cargo-flamegraph uses dtrace, which requires root/sudo.
    if ! command -v flamegraph &>/dev/null && ! cargo flamegraph --version &>/dev/null 2>&1; then
        echo "==> cargo-flamegraph not found — installing..."
        cargo install flamegraph --locked
    fi
    if [[ ! -x /usr/sbin/dtrace ]]; then
        echo "ERROR: dtrace not found at /usr/sbin/dtrace." >&2
        exit 1
    fi
    # Check if we have privilege to run dtrace. On macOS we need sudo (or SIP
    # disabled). Test by probing a no-op; if it fails, explain and exit cleanly.
    if ! sudo -n true 2>/dev/null; then
        echo ""
        echo "INFO: macOS dtrace profiling requires sudo. sudo is not available"
        echo "      non-interactively in this environment."
        echo ""
        echo "      Flamegraph capture is DEFERRED to the Linux CI image (W4-A)."
        echo "      The script is correct and portable — re-run on a Linux runner"
        echo "      or grant sudo access to capture SVGs on macOS."
        echo ""
        echo "      Attribution from the existing dhat profile is available in:"
        echo "        profiles/SCAN-DECODE-ATTRIBUTION.md"
        echo ""
        exit 0
    fi
    echo "==> macOS: profiling via dtrace (sudo available)"
    FLAMEGRAPH_BACKEND="--root"  # cargo-flamegraph flag that adds sudo for dtrace

else
    echo "ERROR: Unsupported OS '$OS'. This script supports Linux (perf) and macOS (dtrace)." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Run flamegraph
# ---------------------------------------------------------------------------
echo "==> Profiling workload: $WORKLOAD"
echo "    Output: $OUT_SVG"

# Run from the repo root so cqlite-perf finds its datasets/ and schemas/.
cd "$REPO_ROOT"

# shellcheck disable=SC2086
cargo flamegraph \
    ${FLAMEGRAPH_BACKEND:-} \
    --bin cqlite-perf \
    --output "$OUT_SVG" \
    -- run \
        --workload "$WORKLOAD" \
        --warmup 0 \
        --duration 8s \
        --trials 1 \
        --concurrency 1

if [[ -f "$OUT_SVG" ]]; then
    echo ""
    echo "==> Flamegraph written: $OUT_SVG"
    ls -lh "$OUT_SVG"
else
    echo "ERROR: flamegraph SVG not produced at expected path: $OUT_SVG" >&2
    exit 1
fi
