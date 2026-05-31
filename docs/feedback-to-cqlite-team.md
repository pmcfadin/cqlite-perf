# Developer feedback: cqlite from a downstream consumer's view

Context: building `cqlite-perf`, an external benchmark harness that depends on
`cqlite-core` by git tag (v0.9.1) and drives `WriteEngine` / `Database` directly.
This is feedback from actually consuming the library cold — not from inside the
repo. Roughly ordered by how much time each cost me.

## 1. Document the feature → API map (highest value)
The public API I needed was gated behind non-default features, and nothing told
me which:
- `WriteEngine` → `write-support` (off by default)
- `Database::flush` / `compact` → `experimental`
- query path → `state_machine` (on by default)

I had to read source to find these. A short table in the README ("want the write
engine? enable `write-support`") would have saved the most time of anything here.
Consider folding `write-support` into defaults.

## 2. Add a "using cqlite-core as a dependency" doc page
One page with: a minimal `Cargo.toml` dep line, the feature table above, and a
~15-line "construct a Mutation and write it" example. The example matters because
the real API differs from what the docs/spec implied (`CellOperation::Write`, not
`Insert`; `Column` requires `is_static`; `WriteEngine::write` is sync). An example
is the fastest way to keep callers correct as the API evolves.

## 3. Provide release binaries
- **CLI (`cqlite`)**: attach prebuilt binaries (macOS arm64+x86_64, Linux
  gnu+musl, Windows) to each GitHub release so users don't need a Rust toolchain.
  Standard for CLI tools; low cost, high reach.
- **Python wheels / npm package**: you already publish these — please verify
  target coverage (manylinux, macOS arm64+x86_64, Windows; all supported Python
  versions) so `pip install` never falls back to a source build, which kills
  adoption.
- **C-linkable lib (.so/.a + cbindgen header)**: only if/when a C/C++ consumer
  appears — defer until there's demand.
- (A prebuilt `cqlite-core` Rust lib isn't possible — no stable Rust ABI — so
  consuming it from source by tag is correct and needs no change.)

## 4. Publish per-tag API docs + changelog visibility
Not on crates.io, so downstreams pin by git tag. A docs.rs-style API doc per tag
(or a hosted equivalent) plus a visible changelog would let a consumer evaluate a
version without checking out the repo.

## 5. Write-path concurrency model
`WriteEngine::write` takes `&mut self` and fsyncs the WAL per write, so concurrent
writers need a Mutex and throughput caps low (~282 ops/sec single-thread in my
harness). Either a batched/async write path or a documented "this is the intended
concurrency model" note would help callers set expectations.

---
Thanks — cqlite was straightforward to link and build once I knew the feature
flags. Items 1–2 are pure documentation and would remove most of the friction.
