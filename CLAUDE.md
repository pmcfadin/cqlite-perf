# cqlite-perf — project guidance for Claude

External macro-benchmark harness for **CQLite**. It depends on `cqlite-core` by
git tag (currently **v0.11.0**). The engine is **not in this repo** — it lives at
<https://github.com/pmcfadin/cqlite> (`pmcfadin/cqlite`, default branch `main`).

## Test roadmap — `TEST_PLAN.md` (read first for any validation work)

[`TEST_PLAN.md`](TEST_PLAN.md) is the canonical plan. Validation follows a
progression — **Unknown → Known (Phase 1) → Improved (Phase 2) →
Regression-locked (Phase 3)** — with a progress dashboard, an engine-version
ledger, the per-version run procedure (Reference A), the correctness matrix with
expected row counts (Reference B), and the triage flow (Reference D).

**Keep it current.** When you validate an engine version, follow Reference A and
**add a row to the ledger**; when a phase exit-criterion is met, **check its box**
and update the dashboard counts. Correctness gates before performance — a
workload that runs but returns wrong rows is a FAIL.

## Filing engine bugs upstream (important)

When benchmarking surfaces a bug in the **engine** — wrong or zero rows, ignored
clauses, crashes, perf cliffs — it must be fixed in `pmcfadin/cqlite`, not here.

> **Never push code to `pmcfadin/cqlite`.** No commits, branches, or PRs to the
> engine repo. Engagement is limited to **issues and comments** — report the bug
> with a repro and let the engine team write the fix.

File it there with `gh`:

```
gh issue list   -R pmcfadin/cqlite --search "<keywords>"   # search first — avoid dups
gh issue create -R pmcfadin/cqlite --title "..." --body "..."
gh issue comment <n> -R pmcfadin/cqlite --body "..."        # add evidence to an existing one
```

Rules of thumb:
- **Search before filing**; if a related issue exists, comment with added evidence
  instead of duplicating.
- **Always include a minimal repro.** An `examples/probe_*.rs` in this repo that a
  maintainer can `cargo run --example` is ideal.
- **State plainly whether it's an engine bug or a harness/data issue**, and give
  the evidence that rules out the harness (e.g. "the stored key bytes decode to
  exactly the generated key; the same code counts N rows on a full scan").
- **Cross-link**: note the upstream issue number on the corresponding cqlite-perf
  tracking issue so the blocker stays visible on this side.

## Environment

- Rust is via rustup but **not on PATH**: prefix shells with
  `export PATH="$HOME/.cargo/bin:$PATH"`.
- `cqlite-core` is a git-tag dep (no local path); cold builds are ~1 min.
- Corpus generation needs Docker (`cassandra:5.0`). Datasets are gitignored —
  only manifests under `datasets/manifests/` are committed. `scripts/gen-corpus.sh`
  bulk-loads via `cqlsh COPY FROM`.
- A workload is **not done until it has run against a real corpus and emitted
  rows** — compiling is not enough.

## Table names must be keyspace-qualified (v0.11.0)

cqlite v0.11.0 (VG7, cqlite #680) keys table identity by `(keyspace, table)`.
An unqualified `FROM basic` no longer resolves on the query path and silently
returns **0 rows** — every query must use `perf.basic` / `perf.wide_rows` /
`perf.collections`. The harness query builders and the `examples/probe_*` repros
are all qualified; keep new ones qualified too.

## Known upstream blockers

The dep is pinned to **`tag = "v0.12.0"`** (cut 2026-06-23, commit `b70a64e8`)
and `runner::CQLITE_VERSION` is `"v0.12.0"`. Baseline re-run on 2026-06-26 (#16,
closed): scan throughput +41% (332k → 470k rows/s), live-heap 79.4 MB; #788/#790
hold on the tag (see below).

### Open at v0.12.0
- **Read p99 under concurrent write load ~2× worse than the interim pre-tag main
  commit** (cqlite **#1143**, performance). Same-machine A/B: `mixed.read_while_write`
  reader p99 ~200 µs (`9054734`) → ~371 µs (v0.12.0), distributions non-overlapping
  across 8 samples. *Isolated* scan throughput *improved* 41% over the same range,
  so this is a contention/tail regression, not a scan-speed one. **Fixed on main
  2026-07-01** (cqlite PR #1347): root cause was #964 flipping the default read
  backend to mmap with `PrefetchMode::Auto` → `MADV_SEQUENTIAL` drop-behind, so
  under write load evicted pages became synchronous major faults on tokio worker
  threads. The fix lands in the **next tag** — validate then and close harness #27.
- **`read.point_lookup` is still an O(rows) full scan** (~173 ms/op for 1 row).
  #755 (BTI offset primitive) and #949 (partition-eq lookup) are in the tag, but
  the within-SSTable single-candidate seek wiring (#953, closed 2026-06-24) and
  its regression fix (#1105, closed 2026-06-26) landed **after** the v0.12.0 cut.
  So sub-ms lookups are pending the next tag — tracked on harness #17.

### Fixed in v0.12.0 (validated on the tag; were validated earlier on interim main)
- **Clustering-key inequality bounds (`>=`, `>`, `<`) now applied** (cqlite
  **#788**). `WHERE pk=? AND ck>=? AND ck<?` returns the slice, not the whole
  partition. `read.clustering_slice` genuinely slices (200 rows/op, was 1000;
  throughput also ~doubled, 1194 → 2358 rows/s). Repro: `cargo run --example
  probe_slice` (all forms → 200).
- **`execute_streaming` no longer materializes the whole result** (cqlite
  **#790**). Live-heap high-water on a 100k-row `read.full_scan` is 79.4 MB on
  the tag (was 194 MB pre-fix), via `memscan --features dhat-heap`; peak RSS
  ~0.9–1.7 GB. Still appears to scale somewhat with row count (not fully O(1)) —
  a row-count scaling test (larger corpus) would confirm; not currently blocking.

### Fixed in v0.11.0 (were blocking in v0.10.0)
- **`WHERE pk = ?` on a TEXT partition key now returns correct rows** (cqlite
  #586 → PR #588, "reconstruct TEXT partition-key columns on the scan path").
  Unblocks `read.point_lookup` (1 row per lookup). Note it's still a full scan
  with residual filtering per lookup (~173 ms/op on v0.12.0) — the O(log n)
  partition seek (#755/#949/#953) is not yet in a tag; see "Open at v0.12.0"
  above. Repro: `cargo run --example probe_point`.
- **`maintenance_step()` no longer panics inside a tokio runtime** (cqlite #587
  → PR #593). `write.compaction` keeps the `spawn_blocking` hop for now; it can
  be simplified to a direct call later.

### Fixed in v0.10.0 (were blocking in v0.9.2)
- `LIMIT` now enforced on the streaming path (cqlite #581 → #582).
- UUID/TIMEUUID `WHERE` returns correct rows (cqlite #583).
- Scan throughput up ~40% on lz4 (~218k → ~311k rows/s), likely the Index.db
  point-lookup work (#584). `write-support` is now a default feature (#558).

## M2 workloads (write + mixed)

Run the whole write+mixed suite + regenerate reports with
`scripts/run-write-mixed.sh`. Workloads added:
- `write.ingest` (WAL-on) / `write.ingest_waloff` (WAL-off) — per-worker engines,
  real concurrency scaling. `write.flush`, `write.compaction`.
- `mixed.read_while_write`, `mixed.open_loop` — readers full-scan the basic
  corpus (a plain scan is the cleaner read-load generator; #586 is fixed so
  point_lookup also works now); writers ingest. Open-loop driving
  with coordinated-omission correction lives in `runner::drive_plan` via the
  per-workload cohort plan (`Workload::work_plan`).
- Write/mixed metrics outside the standard envelope ride in `RunResult.custom`
  (`Workload::custom_metrics` + runner per-cohort emission); goals select them as
  `custom.<key>`. `cqlite-perf report --results <jsonl>` re-renders SUMMARY +
  SCORECARD from an accumulated results.jsonl.

## Commit trailer

```
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```
