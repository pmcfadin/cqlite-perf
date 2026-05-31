# PRD Addendum — Audiences, User Stories & the Goals Scorecard

**Date:** 2026-05-31
**Status:** Addendum to `docs/spec.md` (the build spec). Where this conflicts
with the spec on *purpose*, this document wins; the spec remains authoritative
on *implementation*.

The spec (§1) states *what* cqlite-perf measures but never names *who the
numbers are for* or *what "good" means*. This addendum fixes that. It defines the
two audiences as first-class user stories with acceptance criteria, and
introduces the **goals scorecard** — the mechanism that turns raw measurements
into "are we meeting our targets?"

---

## 1. The product is one tool, two report products

`cqlite-perf` is a **single binary** (`run` / `gen` / `compare` / `datasets` /
`scorecard`). It is *not* a suite of separate tools. The "suite" is two distinct
**report outputs** emitted from the same measurement engine and the same
canonical JSONL:

| Report | Audience | Answers | Artifact |
|--------|----------|---------|----------|
| **Headline** | Users / evaluators | "What performance can I expect?" | `SUMMARY.md` — absolute rows/sec, latency percentiles, codec size-vs-speed, bindings overhead, in human-readable prose + tables |
| **Scorecard** | cqlite dev team | "Are we meeting our targets, and did we regress?" | `SCORECARD.md` — each tracked metric vs. a **declared goal** and vs. the prior version, met/missed/regressed flagged |

Both derive from `reports/<date>-<version>/results.jsonl`. No measurement is run
twice for the two reports; they are two renderings of one dataset.

---

## 2. User stories

### US-1 — Evaluator ("how would this work for me?")

> As someone deciding whether to adopt cqlite, I want trustworthy absolute
> numbers for the workloads I care about — scan throughput, point-read latency,
> the compression size/speed tradeoff, and the overhead of the Python/Node
> bindings vs. native Rust — so I can predict how cqlite behaves on my data and
> in my language.

**Acceptance criteria**
- A1. `SUMMARY.md` reports, for each workload: median throughput (rows/sec and
  ops/sec as appropriate), p50/p99/p99.9 latency, and the cold-vs-warm
  distinction — with units and ≥3 trials + CV shown (no single-run numbers).
- A2. A codec table gives on-disk size and scan rows/sec per codec, so the
  size-vs-speed tradeoff is readable at a glance (SPEC §10).
- A3. A bindings table gives the native-vs-Python-vs-Node overhead ratio
  (SPEC §13).
- A4. Every headline number is reproducible: the report records cqlite version,
  host, dataset checksum, seed, warmup/duration/trials.

### US-2 — Dev team ("are we meeting our goals?")

> As a cqlite maintainer, I want each performance metric checked against a
> declared target and against the previous release, so I can see at a glance
> whether we are meeting our goals and whether a change regressed anything —
> not just raw numbers I have to interpret.

**Acceptance criteria**
- B1. Targets are declared in a committed `goals.toml` (one place, versioned).
- B2. `cqlite-perf scorecard` reads a `results.jsonl` + `goals.toml` and emits
  `SCORECARD.md` marking each metric **MET / MISSED / NO-GOAL**.
- B3. When a baseline `results.jsonl` is supplied, the scorecard also flags
  **REGRESSED** (worse than baseline beyond a threshold) / **IMPROVED**.
- B4. The scorecard exits non-zero if any metric with `enforce = true` is missed
  or regressed, so CI (SPEC §14) can gate on it.
- B5. A metric with no matching goal is reported as NO-GOAL, never silently
  dropped — coverage gaps are visible.

---

## 3. Why goals, not just deltas

The spec's cross-version `compare` (§15) answers "did we change?" but not "are we
where we want to be." A 5% scan regression might still beat target; a version
that never regressed might never have met goal. **Declared targets make findings
actionable** — e.g. the observed 274 MB streaming-scan RSS is only a *miss* if
"<128 MB streaming" is a declared goal (it is; see `goals.toml`). Goals convert
"we produce data" into "we answer the two questions."

---

## 4. Goal definition model

`goals.toml` is a flat list of goal entries. Each names a metric, a selector
(which results it applies to), a comparison, a target, and whether it gates CI.

```toml
# goals.toml — declared performance targets for cqlite.
# A goal matches a result when every set selector field equals the result's.
# Comparison is read as "<metric> <op> <target>" — the goal is MET when true.

[[goal]]
name        = "basic scan decode throughput"
metric      = "throughput.rows_per_sec"   # dotted path into the metric envelope
op          = ">="                         # >=, <=, >, <
target      = 1_000_000                    # 1M rows/sec
# selector: only results matching ALL of these are judged by this goal
workload    = "read.full_scan"
schema      = "basic"
tier        = "S"
enforce     = true                         # CI gates on this one

[[goal]]
name        = "point lookup p99 latency"
metric      = "latency_us.p99"
op          = "<="
target      = 500                          # 500 µs
workload    = "read.point_lookup"
enforce     = true

[[goal]]
name        = "streaming scan stays memory-bounded"
metric      = "resource.peak_rss_bytes"
op          = "<="
target      = 134_217_728                  # 128 MB (SPEC §7)
workload    = "read.full_scan"
enforce     = false                        # observed 274 MB — track, don't gate yet

[[goal]]
name        = "regression budget on scan throughput"
metric      = "throughput.rows_per_sec"
op          = ">="
# relative to baseline rather than an absolute target:
regression_max_pct = 10.0                  # fail if >10% worse than baseline
workload    = "read.full_scan"
enforce     = true
```

Selector fields (all optional; omitted = matches any): `workload`, `schema`,
`tier`, `codec`, `distribution`, `concurrency`, `cache`.

A goal carries **either** an absolute `target` (with `op`) **or** a
`regression_max_pct` (judged against a baseline), not both.

---

## 5. Scorecard output (SCORECARD.md)

```
# cqlite-perf — goals scorecard
cqlite v0.9.2 · baseline v0.9.1 · host aarch64 (10 cores)

| Goal | Selector | Metric | Target | Actual | vs base | Status |
|------|----------|--------|-------:|-------:|--------:|--------|
| basic scan decode throughput | full_scan/basic/S | rows/sec | ≥1,000,000 | 151,000 | +2% | ❌ MISSED |
| point lookup p99 latency | point_lookup | p99 µs | ≤500 | 240 | -5% | ✅ MET |
| streaming scan memory-bounded | full_scan | peak RSS | ≤128 MB | 274 MB | — | ❌ MISSED (not enforced) |

2 enforced goal(s) failed → exit 1
```

---

## 6. Milestone reprioritization (mission-first)

The mission is **produce real data** for two audiences (US-1, US-2). Milestones
are therefore reordered by how directly they serve that mission, not by the
spec's original numbering. The spec's §16 build order is superseded here.

**Core (required to meet the mission):**

- **M1 — read suite + datasets.** Real read numbers for both audiences. Mostly
  done; tail remaining = generate the codec-sweep corpora + `type_heavy` /
  `wide_partition` corpora, add `read.clustering_slice`.
- **M2 — write + mixed suites.** Real write/flush/compaction + mixed numbers;
  completes workload coverage. Biggest remaining data-coverage gap.
- **M5 — profiling.** `dhat` allocation tracking, flamegraphs, and the <128 MB
  RSS assertions. This is the *why* behind the numbers: it turns a scorecard
  **miss** into a **diagnosis** (e.g. the observed 310 MB streaming-scan RSS is a
  flagged miss today — M5 explains the allocation that causes it, which is what
  makes US-2 actionable).

**Deferred (valuable, but not required for the mission):**

- **M3 — bindings overhead.** A real user concern, but secondary to establishing
  native-engine truth first. Pick up once the core data story is solid.
- **M4 — CI + cross-version compare.** Automation and release-over-release
  gating. Its *cheap, high-value* half already exists: the scorecard's
  `regression_max_pct` + `--baseline` answers "did we regress vs. last release"
  without M4's CI runners or two-checkout build machinery. The *expensive* half
  (self-hosted runners, dual builds) is deferred.

**Order of work:** finish M1 tail → **M2** → **M5**. M3/M4 only if the mission
later demands language reach or automation.

### M1.5 — the goals scorecard (done)

Slotted in ahead of the above because it changes how every number is *consumed*:
`goals.toml` schema + `cqlite-perf scorecard` + `SCORECARD.md` emitter (absolute
goals; regression flagging when a baseline is supplied). Targets in `goals.toml`
are seeded placeholders — **the dev team owns calibrating them**, and that act of
writing down a number is itself the deliverable US-2 asks for.
