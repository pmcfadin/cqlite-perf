# ci/Dockerfile — reproducible Linux build for cqlite-perf (issue #8, W4-A)
#
# Base: rust:1.88 (matches rust-toolchain.toml channel = "1.88.0").
# Arch: linux/arm64 (matches Docker Desktop on Apple Silicon / aarch64 CI).
#       On x86-64 CI, build with --platform linux/amd64.
#
# Datasets are NOT baked in. Mount them at runtime:
#   docker run --rm -v "$PWD/datasets:/work/datasets:ro" cqlite-perf-ci validate
#
# Flamegraph support:
#   perf is available inside the container (linux-perf from the debian package).
#   To capture a flamegraph on a real Linux host (not Docker Desktop / macOS VM)
#   mount the kernel debug symbols and run with --privileged:
#   docker run --rm --privileged -v /sys/kernel/debug:/sys/kernel/debug ... \
#     cqlite-perf-ci <flamegraph commands>

FROM rust:1.88

# ── system dependencies ────────────────────────────────────────────────────────
# linux-perf: `perf` tool for flamegraph capture.
# git: required by cargo to fetch the git-dep (cqlite-core rev pin).
# ca-certificates: HTTPS to github.com during cargo fetch.
# pkg-config + libssl-dev: openssl-sys transitive dep (cargo build time).
RUN apt-get update && apt-get install -y --no-install-recommends \
        linux-perf \
        git \
        ca-certificates \
        pkg-config \
        libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Make `perf` available as `perf` (debian installs it as perf_<version>)
RUN ln -sf "$(ls /usr/bin/perf_* 2>/dev/null | head -1)" /usr/local/bin/perf 2>/dev/null || true

# flamegraph cargo subcommand (optional — baked in so `cargo flamegraph` works
# without a network connection at runtime; slow to install, cached in layers).
RUN cargo install flamegraph --locked 2>/dev/null || \
    cargo install flamegraph || true

WORKDIR /work

# ── source copy (committed files only; datasets are bind-mounted at runtime) ───
# Copy the full source tree: Cargo.toml/lock, src/, schemas/, goals.toml, scripts/
# .dockerignore excludes datasets/, reports/, profiles/, target/, .git/
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src/       src/
COPY schemas/   schemas/
COPY goals.toml ./
COPY scripts/   scripts/
COPY configs/   configs/

# ── build release binary ───────────────────────────────────────────────────────
# cqlite-core features are declared in Cargo.toml (write-support, cli-helpers).
# A plain `cargo build --release` picks them up correctly.
# Network is needed here to fetch the git-rev dep from github.com.
RUN cargo build --release

# ── entrypoint ─────────────────────────────────────────────────────────────────
# Default: run the correctness gate.
# Override at `docker run` time to run perf smoke, report, etc.
ENTRYPOINT ["/work/target/release/cqlite-perf"]
CMD ["validate"]
