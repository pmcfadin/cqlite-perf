# CQLite performance testing strategy — the ideal suite

Status: adopted 2026-07-02 · tracking epic: [#30](https://github.com/pmcfadin/cqlite-perf/issues/30)
· companion docs: [TEST_PLAN.md](../TEST_PLAN.md) (the per-version procedure this
strategy feeds), [SPEC.md](../SPEC.md) (harness design).

This document describes the complete performance-testing program for CQLite:
what is measured, at which layer, on what cadence, in which environment, and
how a measurement becomes an enforced regression gate. It covers what already
exists, what is being built (epic #30), and the end state.

---

## 1. Principles

These are the rules every tier follows. They come from lessons this project has
already paid for.

1. **Correctness gates before performance.** A workload that runs fast but
   returns wrong rows is a FAIL, full stop. Every tier — including the
   distributed one — runs a row-count/parity gate before any timing is
   recorded. (This is how cqlite #586, #788, and the keyspace-qualification
   silent-zero-rows behavior were caught.)
2. **The harness is an external oracle.** It consumes the engine exactly as a
   user does: git-tag dependency, public API, real corpora. It never lives in
   the engine's workspace and never pushes code upstream — **issues with
   repros only**. Independence is what makes its findings trustworthy and lets
   it catch packaging/default-feature/behavior-change bugs that in-tree tests
   compiled against the workspace cannot.
3. **Macro complements micro; neither substitutes for the other.** Upstream's
   Criterion PR gate catches per-function regressions before merge. It
   structurally cannot see emergent behavior: the v0.12.0 read-p99 regression
   (cqlite #1143 — mmap drop-behind turning cache hits into synchronous major
   faults *under concurrent write load*) sailed through micro-benches while
   isolated scans got 41% **faster**. Only a mixed-workload, tail-latency,
   system-level harness sees that class of bug.
4. **Data-grounded enforcement, via the ladder.** No guessed absolute targets.
   Every gate goes: *characterize* (measure, non-enforcing) → *calibrate* (set
   budgets from ≥3 measured runs, with rationale comments in `goals.toml`) →
   *enforce* (`enforce = true`, red CI on breach). This is how the T1 gates
   went live (#19 → #23) and how the soak trend gates will (#32).
5. **Absolute numbers need clean rooms; relative numbers don't.** Absolute
   baselines follow Reference C (bare metal, quiesced, content-addressed
   corpora, never compare across host signatures). But *self-relative*
   assertions — PR-vs-main on the same runner (upstream's gate), a soak run's
   last hour vs its own first hour — are robust to host variance and are
   therefore legitimate on shared CI runners. Choosing the right assertion
   style is what makes each tier cheap enough to run on its natural cadence.
6. **Every surprise becomes an upstream issue with a minimal repro** —
   including *good* surprises (confirmations are findings; file the evidence).
   Cross-link both directions so blockers stay visible.

---

## 2. The tier model

Seven tiers, ordered by scope and cost. Each has one job; together they cover
the space from a single function to a multi-node cluster.

| Tier | Scope | What it catches | Cadence | Environment | Assertion style | Status |
|---|---|---|---|---|---|---|
| **T0** micro | one function/path | per-function regressions, pre-merge | per engine PR | GH runner | self-relative (PR vs main, same runner) | ✅ upstream `perf-regression.yml` |
| **T1** macro gates | full workloads, embedded engine | correctness breaks; macro throughput/latency regressions on engine bumps | per harness PR + master push | GH runner | hard correctness gate + baseline-relative enforced goals (30% budget) | ✅ `ci.yml` (#8, #20, #23) |
| **T2** main-tracking | same as T1, vs engine `main@HEAD` | between-tag regressions *and* early confirmation of fixes | nightly | GH runner | hard correctness gate; perf non-enforcing vs tag baseline | ✅ `main-tracking.yml` (#29) |
| **T3** soak / long-burn | hours of sustained load | drift: leaks, fragmentation, compaction debt, tail degradation over time | weekly (2 h) + per-tag authoritative (4 h+, bare metal) | GH runner + Mac | **trend properties** (self-relative: retention, slope, drift ratio) | ⬜ epic #30 (#31–#34) |
| **T4** large-corpus baselines | 25M-row (M-tier) absolute numbers | scale effects invisible at 100k rows; public-claim substantiation | per tag | bare metal | absolute, Reference C clean room | ⬜ #24 |
| **T5** distributed / service | cqlite-flight + Trino on a real Cassandra cluster | Arrow stream throughput, merge cost, token-range scaling, E2E vs stock connector, co-location impact | spike, then per-milestone | AWS via easy-db-lab | correctness parity gate + within-cluster comparisons | ⬜ #36 (spike) |
| **T6** bindings overhead | Python / Node vs native | FFI/serialization tax per surface | per tag | bare metal | ratio vs native (self-relative) | ⬜ #7 (deferred) |

Reading the table: cost grows downward, cadence slows downward, and the
assertion style shifts from "hard gate" to "measured comparison" — enforcement
concentrates where the signal is cheapest and least noisy, measurement-only
where environments are expensive.

### T0 — micro-benchmarks (upstream; exists)

Criterion read+write benches run on both the PR and `main` on the same runner;
fails if any tracked bench's median regresses beyond the threshold in
`cqlite-core/benches/perf-gate.json`. Owned entirely by the engine repo. The
harness's only interaction: when a T1–T5 finding is fixed upstream, suggest a
micro-bench that would have caught it earlier, when one is expressible.

### T1 — macro gates (exists)

`ci.yml` on every harness PR and master push (which is how engine bumps land):
content-addressed corpus provisioning → `datasets --check` → **`cqlite-perf
validate`** (Reference B row counts, hard gate) → read smoke → `scorecard
--enforce` with baseline-relative goals (30% budget on `read.full_scan`
throughput and `read.point_lookup` p99). On PRs, the baseline is `master`
re-measured in a git worktree **on the same runner, same job** (issue #39) —
principle 5 in action: cross-runner comparison on GitHub's heterogeneous
fleet produced spurious ~50–60% "regressions" from pure hardware variance,
so the enforced comparison had to become machine-invariant. The
`linux-baseline-master` artifact from prior master pushes is kept only as
informational trend context, never gated on.

### T2 — main-tracking (exists)

Nightly `main-tracking.yml`: rev-pins to engine `main@HEAD`, hard correctness
gate, smoke including `mixed.read_while_write` (the workload class that caught
#1143), non-enforcing scorecard vs the tag baseline into the job summary. Dual
purpose: regressions on main surface within a day instead of at the next tag
re-baseline, and **fixes are confirmed early** — its shakedown run confirmed
sub-ms point lookups (4,378 ops/s, p99 263 µs, vs ~3 ops/s / p99 ~400 ms on
the tag) the day the seek wiring question came up.

### T3 — soak / long-burn (epic #30; the current build-out)

Everything above measures seconds-scale windows. T3 measures **hours**, which
is the only way to see time-dependent behavior:

- **Memory drift.** The #790 fix left live-heap "not fully O(1) in rows"; a
  2-hour RSS series with a least-squares slope answers whether there is a
  slow-growth term. (dhat stays at T1 — its overhead distorts long runs; RSS
  slope is the leak signal at this tier.)
- **Compaction debt.** Sustained WAL-on ingest with maintenance cycling: does
  throughput retention hold, or do SSTables pile up until read amplification
  bites? Directly feeds upstream #905 (compaction manager).
- **Tail drift.** Is hour-4 reader p99 under mixed load the same as minute-1?

Mechanics (details in the child issues):

- **#31** — the runner grows `--snapshot-interval`: per-window (reset-on-
  snapshot) histograms per cohort, RSS sampling, a sidecar `intervals.jsonl`
  series, and derived **trend metrics** on the final result
  (`custom.trend.throughput_retention`, `.rss_slope_mb_per_h`, `.rss_max_mb`,
  `.windows`, `.p99_drift_ratio` for single-cohort runs, or
  `.read_p99_drift_ratio` / `.write_p99_drift_ratio` for mixed workloads with
  `read`/`write` cohorts) — quartile-based so one noisy window can't flip a
  verdict, and emitted through `RunResult.custom` so the existing scorecard
  selects them with zero schema changes.
- **#32** — trend goals in `goals.toml`, born `enforce = false`, each with a
  PLACEHOLDER rationale comment. Calibration ladder: collect ≥3 weekly
  `soak.yml` (#34) runs → derive each target from the measured spread
  (retention floor = measured − 3×stddev, min 0.90; drift ceiling = measured +
  margin; slope ceiling = measured + 3×stddev with an absolute cap) → replace
  the PLACEHOLDER target with the data-grounded one in a PR citing the runs →
  flip `enforce = true` — the same ladder as principle 4 above and the T1
  gates (#19 → #23). `custom.trend.*` keys only exist on `--snapshot-interval`
  runs, so these goals render `Status::NoData` (not a failure) on every T1/T2
  run that doesn't set that flag — `scorecard --enforce` in `ci.yml` is
  unaffected. A follow-up issue ("flip soak goals to enforce", citing the
  calibration runs) gets filed once #34 has accumulated that history.
- **#33** — two profiles: `soak.mixed` (2 h `mixed.read_while_write`, conc 8,
  60 s windows — the #1143-class detector) and `soak.ingest` (2 h sustained
  WAL-on ingest + maintenance — the compaction-debt detector); plus
  `scripts/run-soak.sh` and TEST_PLAN **Reference E** (cadence, budgets,
  triage, ledger extension).
- **#34** — `soak.yml`, Saturdays, default engine = pinned tag (isolating
  time-dependent behavior from engine churn — churn is T2's job), optional
  `rev` input for on-demand soaks of main, trend table in the job summary,
  90-day artifacts.
- **#35** — the attribution gap: soak *detects* drift but can't *attribute*
  it without cheap engine counters (SSTable count/bytes, memtable bytes, WAL
  bytes, compaction backlog). That becomes an upstream API ask
  (`Database::stats()`), cross-linked to #905 and #1352; when it ships, the
  counters ride the interval series as `custom.engine.*`.

Why weekly-on-shared-runners is sound: every T3 assertion is self-relative
(the run against its own first quartile), so runner variance largely cancels;
absolute numbers from soak runs are informational only. The authoritative
soak (4 h+, quiesced Mac, per engine tag) anchors the ledger.

### T4 — large-corpus absolute baselines (#24)

The S tier (100k rows) is deliberately small enough for CI. Some questions only
answer at scale: does scan throughput hold at 25M rows, where do BTI/index
structures start paying off or hurting, what do memory high-waters look like
when the corpus dwarfs page cache. Per-tag, bare metal, full Reference C
protocol. Also the substantiation layer for any public performance claim —
numbers quoted outside the repo come from this tier or T5, nowhere else.

### T5 — distributed / service benchmarking (#36 spike)

`cqlite-flight` is the engine's distributed data plane: an Arrow Flight server
co-located with each Cassandra node, k-way-merging that node's SSTables on
read (LWW + tombstones), applying token-range/predicate/projection filters,
streaming Arrow record batches — the backend of the Trino connector, which
assigns each token range to one replica's Flight endpoint. Its performance
questions are inherently multi-node and are currently measured **nowhere**.

**[easy-db-lab](https://github.com/rustyrazorblade/easy-db-lab)** is the
provisioning layer: lab Cassandra clusters (2.2–trunk) on EC2 from a packer
AMI that ships with cassandra-easy-stress, Prometheus + Grafana dashboards,
OpenTelemetry, async-profiler, and bcc-tools — and **K3s on every node**,
which is how cqlite-flight (it already has a Dockerfile) and Trino deploy.

Topology:

```
easy-db-lab up:   3 × Cassandra 5.0 (NVMe-backed) + 1 stress/coordinator node
per C* node:      cqlite-flight container (K3s), mounting the node's data dir + snapshots/
coordinator:      Trino (K3s) with two catalogs on the same cluster & data:
                  cqlite connector (in.mcfad:cqlite-trino)  vs  stock cassandra connector
loading:          cassandra-easy-stress (KeyValue + wide-partition profiles), ~100M rows,
                  nodetool flush + named snapshot (Flight reads SSTables; snapshots give
                  a consistent file set while Cassandra compacts underneath)
```

The six measurement axes (full detail in #36):

1. **Single-endpoint DoGet throughput** — rows/s and Arrow MB/s per node, cold
   vs warm page cache.
2. **Merge cost vs SSTable count** — the per-read k-way merge is the novel
   cost; measure at 1/4/16 SSTables per table, pre/post `nodetool compact`.
3. **Token-range parallelism** — does a 3-node Trino full scan approach 3× a
   single endpoint?
4. **End-to-end Trino: cqlite connector vs stock Cassandra connector** — the
   headline comparison, same cluster, same data, same queries (count, filtered
   scan, narrow projection, partition lookup).
5. **Co-location impact** — cassandra-easy-stress foreground read/write p99
   with and without a concurrent Flight scan on the node: the cluster-level
   analog of the #1143 finding, watched on the pre-wired dashboards.
6. **Snapshot semantics under churn** — snapshot reads while compaction runs
   underneath: correctness first, then latency variance.

Correctness gate first, as always: Trino-via-Flight row counts and checksum
aggregates must match CQL `SELECT` ground truth before any timing counts.
Profiling: perf/bcc-tools for the Rust Flight server, async-profiler for the
JVM side — all in the AMI. Cost discipline: lab-dedicated AWS account,
same-session teardown, actual dollar cost recorded in the runbook so the
recurring-cadence decision (per-tag? quarterly?) is made from data.

### T6 — bindings overhead (#7, deferred)

Native vs Python vs Node overhead ratios per workload class (SPEC §13). Ratio
assertions (self-relative), bare metal. Un-defers once the core data story is
stable; increasingly relevant as the engine grows Python/Node/Trino/Flight
surfaces faster than measurement for them.

---

## 3. Cross-cutting machinery

**Corpora.** Content-addressed (SHA-256 manifests committed; data gitignored;
`datasets --check` is a hard gate everywhere). S tier (100k rows ×
basic/wide_rows/collections × codec sweep) for T1–T3; M tier (25M) for T4;
cluster-scale data for T5 is generated in-lab by cassandra-easy-stress, with
profiles chosen to mirror the harness schemas so findings translate. Verified
reproducible: a fresh generation on a CI runner reproduces the committed SHA.

**Metrics envelope.** Every run emits the standard envelope (throughput,
latency p50/p99/p999/max, resource peaks) to `results.jsonl`; workload-specific
metrics ride `RunResult.custom` (`custom.<key>` in goals). T3 adds the
`intervals.jsonl` sidecar series and `custom.trend.*` summaries. Open-loop
cohorts use coordinated-omission-corrected driving (`runner::drive_plan`).

**Scorecard + goals.** One mechanism for every tier: `goals.toml` entries
select results (workload/schema/tier), assert either an absolute `op`/`target`
or a `regression_max_pct` vs a baseline, and `scorecard --enforce` turns
breaches into non-zero exits. Goals carry rationale comments citing the
measured numbers that set them. The enforcement ladder (§1.4) governs every
new goal.

**Profiling toolkit.** dhat (allocation attribution; found the HashMap-rehash
≈40% of scan decode), `scripts/flamegraph.sh` (perf on Linux, dtrace on macOS;
SVG capture pending a privileged runner, #25), and in-lab async-profiler +
bcc-tools for T5. Profiles attach to upstream filings per Reference D.

**The improvement loop.** Each tier feeds the same loop: characterize → file
upstream with repro/profile → validate the fix on an interim rev → promote to
a tag baseline → recalibrate → lock with a gate. Seven engine bugs have gone
through it to fixed (LIMIT #581, UUID WHERE #583, TEXT PK #586, tokio panic
#587, clustering bounds #788, result materialization #790, p99-under-write
#1143). T3 and T5 exist to widen the intake of that loop, not to change it.

---

## 4. Cadence summary

| When | What runs | Enforced? |
|---|---|---|
| every harness PR / engine bump | T1: validate + smoke + scorecard `--enforce` | ✅ correctness + calibrated perf goals |
| nightly | T2: main-tracking vs engine main | correctness ✅ · perf informational |
| weekly (Sat) | T3: 2 h soak, mixed + ingest profiles | correctness ✅ · trends informational → enforced after calibration |
| per engine tag | Reference A full baseline + authoritative 4 h soak (Mac) + T4 M-tier + T6 ratios | ledger + recalibrated goals |
| per milestone / on demand | T5 lab run (easy-db-lab), flamegraph captures, main-rev soaks | measured, findings filed |

## 5. Roadmap → issues

| Item | Issue(s) |
|---|---|
| Soak tier (runner series → trend goals → profiles/runbook → weekly CI) | epic #30: #31 → #32 → #33 → #34 |
| Engine observability counters (attribution) | #35 → upstream filing (cross-links cqlite #905, #1352) |
| Distributed Flight/Trino spike on easy-db-lab | #36 → runbook `docs/flight-bench-runbook.md` |
| M-tier absolute baselines | #24 |
| Flamegraph SVG capture (privileged runner) | #25 |
| Bindings overhead | #7 |
| Point-lookup + p99 validation at next tag | #17, #27 |
