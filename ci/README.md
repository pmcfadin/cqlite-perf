# Linux CI image (W4-A, issue #8)

Reproducible Linux container image for running the cqlite-perf correctness gate
and perf smoke. Supports `linux-perf` flamegraph capture.

## Architecture note

The image is built for `linux/arm64` by default (matching Docker Desktop on
Apple Silicon / aarch64-based CI runners). Numbers from this image are from a
**virtualized Linux environment** (Docker Desktop on macOS), not bare metal.
Headline perf numbers and flamegraphs for publication should be captured on a
bare-metal Linux host or a dedicated Linux CI runner. The image is correct for
correctness gating regardless of the host.

## Build the image

Run from the repo root:

```sh
docker build -f ci/Dockerfile -t cqlite-perf-ci .
```

Expected build time: ~3-5 minutes cold (cargo fetches cqlite-core git dep and
compiles ~150 crates). Subsequent builds use the Docker layer cache and are fast
unless Cargo.toml/Cargo.lock change.

For explicit platform targeting:
```sh
# arm64 (default, Apple Silicon / Graviton)
docker build --platform linux/arm64 -f ci/Dockerfile -t cqlite-perf-ci .

# x86-64
docker build --platform linux/amd64 -f ci/Dockerfile -t cqlite-perf-ci .
```

## Run the correctness gate

Datasets live on the host at `./datasets/` (gitignored). Mount them read-only:

```sh
docker run --rm \
  -v "$PWD/datasets:/work/datasets:ro" \
  cqlite-perf-ci validate
```

Expected output (all 3 corpora present):
```
• read-basic-S-lz4
  ✅ 100000 (= 100000)  scan
  ...
11 checks, 0 failed, 0 corpus(es) skipped
✓ correctness gate passed
```

If some corpora are absent the gate skips them with a warning and continues.
If ALL corpora are absent it exits non-zero (`no corpora present`). Generate
them first with `scripts/gen-corpus.sh` (requires Docker + Cassandra 5.0).

## Run a perf smoke

Any `cqlite-perf run` flags work. Example — short full-scan smoke:

```sh
docker run --rm \
  -v "$PWD/datasets:/work/datasets:ro" \
  cqlite-perf-ci \
  run --workload read.full_scan --duration 3s --warmup 1s --trials 1
```

To capture the results.jsonl from the container, add a volume for reports:

```sh
docker run --rm \
  -v "$PWD/datasets:/work/datasets:ro" \
  -v "$PWD/reports:/work/reports" \
  cqlite-perf-ci \
  run --workload read.full_scan --duration 10s --warmup 5s --trials 3
```

## Flamegraph capture (bare-metal Linux only)

`linux-perf` and `cargo flamegraph` are baked into the image. On a real Linux
host (not Docker Desktop on macOS — kernel perf events are not available in the
macOS hypervisor) run with `--privileged` and mount the debug fs:

```sh
docker run --rm --privileged \
  -v /sys/kernel/debug:/sys/kernel/debug \
  -v "$PWD/datasets:/work/datasets:ro" \
  -v "$PWD/profiles:/work/profiles" \
  --entrypoint bash \
  cqlite-perf-ci \
  -c "cargo flamegraph --bin cqlite-perf -- run --workload read.full_scan --duration 10s"
```

On Docker Desktop / macOS the `perf` binary is installed but kernel events are
not exposed by the hypervisor. Use `dhat` heap profiling as an alternative:
```sh
# host build with dhat-heap feature, then inspect dhat-heap.json
cargo build --release --features dhat-heap
```

## Image details

- Base: `rust:1.88` (matches `rust-toolchain.toml`)
- cqlite-core: git dep pinned to rev `9054734` (pre-v0.12.0, see CLAUDE.md)
- Included at build time: `Cargo.toml`, `Cargo.lock`, `src/`, `schemas/`,
  `goals.toml`, `scripts/`, `configs/`
- NOT included: `datasets/` (bind-mounted at runtime), `reports/`, `target/`
- Entrypoint: `/work/target/release/cqlite-perf` (default cmd: `validate`)
