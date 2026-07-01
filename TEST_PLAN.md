# cqlite-perf — test roadmap

The harness exists to move a cqlite engine version along one path:

> **Unknown → Known → Improved → Regression-locked**

1. **Phase 1 — Unknown → Known.** Establish ground truth: what does the engine
   correctly do, and what are its real numbers?
2. **Phase 2 — Improvement.** Close the gap between known-current and target —
   find bottlenecks, file them upstream with repros, recalibrate targets from
   data, validate fixes.
3. **Phase 3 — Regression-locked.** Lock in known-good and catch backsliding
   automatically on every version bump.

Guiding rule across all phases: **a workload that runs is not a pass.** It must
emit the *correct* rows (a fast full-partition scan is not a clustering slice)
and its numbers must be reproducible. Correctness gates before performance.

## Progress dashboard

Single source of truth for "where are we." Update it when a phase exit-criterion
is checked (in its phase section below) and when an engine version completes the
Reference A run procedure (add a ledger row).

| Phase | Status | Exit criteria met |
|---|---|---:|
| 1 · Unknown → Known | ✅ done | 4 / 4 |
| 2 · Improvement | ✅ done | 3 / 3 (goals recalibrated #19; v0.12.0 tag re-baseline #16 — loop repeats per engine version) |
| 3 · Regression-locked | ✅ done | 3 / 3 (correctness gate live in CI #8; perf-regression gate live in CI #23) |

Legend: ⬜ not started · 🟡 in progress · ✅ done.

### Engine version ledger

One row per engine version taken through Reference A. "Correctness" = the
Reference B matrix passed; "Perf" = a baseline run is recorded under `reports/`.

| Engine | Date | Correctness | Perf | Notes |
|---|---|:--:|:--:|---|
| v0.11.0 | 2026-06-16 | ✅ | ✅ | keyspace-qualified change surfaced; found cqlite #788/#790 |
| main@9054734 | 2026-06-16 | ✅ | ✅ | #788/#790 validated; read live-heap 79.5 MB |
| v0.12.0 | 2026-06-26 | ✅ | ✅ | re-pinned `rev → tag` (#16); scan +41% (332k→470k rows/s), live-heap 79.4 MB; #788/#790 hold. Found read-p99-under-write-load ~2x regression → cqlite #1143 (fixed upstream 2026-07-01 via #1347, post-tag — validate at next tag). point_lookup still full-scan (seek wiring #953 post-tag) |

---

## Phase 1 — Unknown → Known (characterization)

**Goal:** for a given engine version, produce a *known-good baseline* — a correct
row-count for every workload, a recorded perf number for every workload/tier, a
memory profile, and a bottleneck attribution. Every surprise becomes an issue.

**Activities**
- **Correctness discovery** — run the probe suite (Reference B), record actual
  vs expected row counts. This is how #586 (PK reads) and #788 (clustering
  bounds) were found and how the keyspace-qualified-name change surfaced.
- **Performance baseline** — full read + write + mixed suite (Reference A step 4),
  3 trials, CV < 0.10. Record per workload/tier.
- **Memory decomposition** — dhat build, profile `read.full_scan`; record
  live-heap `t-gmax` and attribute it (this is how we learned RSS overstated
  memory ~11× and the scan materializes the result).
- **Bottleneck attribution** — CPU flamegraph the scan-decode path (#6) to learn
  where throughput actually goes (currently *unknown*).

**Exit criteria**
- [x] Every workload has a *known-correct* row count in Reference B.
- [x] Every workload/tier has a recorded perf + memory baseline. *(S tier; streaming-heap scaling settled O(1) via `memscan` probe, #18 — M/L corpus optional, W2-C)*
- [x] Scan-decode bottleneck attributed (decode vs decompress vs alloc). *(#6 — dhat attribution: HashMap rehash in `parse_row_data_with_offset` ~40%, `parse_block` ~13%, lz4 ~4%, `parse_cell_value_schema_order` ~3.6%; flamegraph script ready for Linux `perf` capture)*
- [x] All deviations filed upstream with repros and cross-linked.

**Open work:** #25 (flamegraph SVG capture on a privileged Linux runner — the
attribution itself landed via dhat, #6), #24 (optional M-tier corpus for
large-scale scan numbers; the O(1)-streaming question was settled without it, #18).

---

## Phase 2 — Improvement

**Goal:** drive the known numbers toward target. We do **not** edit the engine
(issues only) — the harness drives improvement by producing repros + profiles,
recalibrating targets from data, and validating each fix as it lands.

**The improvement loop** (repeat per bottleneck):
1. Characterize the gap (Phase 1 output) — e.g. point lookups full-scan at ~400 ms.
2. File upstream with a minimal repro + profile (e.g. cqlite #755 index seek).
3. When a fix lands, pin the `rev`, re-run the targeted probe/workload, confirm
   the gain (e.g. #790: live-heap 194 → 79.5 MB; #788: slice 1000 → 200 rows/op).
4. Re-baseline (Reference A) and recalibrate the target from the new data.

**Activities**
- Turn "known bad" into upstream issues (done: #788, #790; open driver: #17 read
  index seek / cqlite #755).
- **Recalibrate targets from measured data** (#19) — retire guessed absolutes
  (1M rows/s, 500 µs p99) the way memory was retargeted (#5 → live-heap ≤ 256 MB).
- Validate fixes via the interim `rev`-pin loop; promote to a tag baseline (#16).

**Exit criteria**
- [x] Top bottlenecks each have an upstream issue with a repro/profile. *(#788, #790; read seek #17→cqlite #755)*
- [x] All goals are data-grounded (property or baseline-relative), no placeholders. *(#19 — memory property-based; throughput/p99 baseline-relative, CI-enforced via #23)*
- [x] Every landed fix validated and re-baselined to a tag. *(#788/#790 validated on the v0.12.0 tag; re-baseline done #16)*

**Open work** (the improvement loop repeats per engine version):
- #17 point-lookup seek — engine wiring (cqlite #953 + regression fix #1105)
  landed on main **after** the v0.12.0 cut; validate at the next tag.
- #27 read-p99-under-write-load — upstream cqlite #1143 **fixed on main
  2026-07-01** (PR #1347: mmap `Auto` prefetch no longer issues the
  `MADV_SEQUENTIAL` drop-behind); validate at the next tag.
- #7 bindings overhead (deferred until the core data story is solid).

---

## Phase 3 — Regression-locked

**Goal:** make backsliding impossible to miss. A version bump auto-runs
correctness + performance regression and fails loudly on any backslide.

**Activities**
- **Asserted correctness gate** — ✅ shipped (#20): `cqlite-perf validate`
  asserts Reference B and exits non-zero on any mismatch. Still needs to *run in
  CI* (#8) to fully close the exit criterion below.
- **Baseline-relative perf gates** — enforce throughput/latency within X% of the
  recorded baseline (the scorecard already supports `--baseline` +
  `regression_max_pct`; flip `enforce = true` on calibrated goals).
- **CI automation** (#8) — run the correctness gate + perf suite on each engine
  version, on a native Linux runner (not Docker-on-Mac — see Reference C), with
  cross-version compare.

**Exit criteria**
- [x] Correctness mismatch fails CI (non-zero exit), not a human reading a table. *(`.github/workflows/ci.yml` — `cqlite-perf validate` is a hard gate; verified green on PR #22, runs 27667412017 + 27667818006)*
- [x] Perf regression beyond budget fails CI against the recorded baseline. *(`scorecard --enforce` gating ships in #23: two baseline-relative enforced goals — `read.full_scan` throughput and `read.point_lookup` p99, 30% budget — conditional on a prior `linux-baseline-master` artifact; first run yields NoData → not a failure)*
- [x] Every engine bump runs the gate automatically. *(ci.yml fires on `pull_request` + `push: master`; an engine bump is a Cargo.toml/Cargo.lock change that lands that way)*

**Open work:** flamegraph SVG capture on a privileged Linux runner (#25, script
ready). All three Phase 3 exit criteria are now met.

**Between-tag tracking:** the CI gates only fire when the pin changes, so a
regression on engine `main` stays invisible until the next tag re-baseline
(cqlite #1143 was caught that way — at #16, days after it landed). The nightly
**main-tracking** workflow (`.github/workflows/main-tracking.yml`) closes that
gap: it rev-pins the harness to engine `main@HEAD`, runs the correctness gate
(hard fail) and a perf smoke including `mixed.read_while_write`, and renders a
non-enforcing scorecard against the latest tag baseline.

---
---

# Reference toolbox

The mechanisms every phase uses.

## Reference A — Per-version run procedure

0. **Pin.** Tag → `Cargo.toml` `tag = "vX.Y.Z"`; interim commit → `rev = "<sha>"`.
   `cargo update -p cqlite-core`. Set `runner::CQLITE_VERSION`.
1. **Build gate.** `cargo build --release` and `--features dhat-heap` both clean.
2. **Correctness gate (blocking).** `cqlite-perf validate` — asserts the
   Reference B row counts, non-zero exit on any mismatch → STOP, triage.
3. **Smoke.** `cqlite-perf run --config configs/read-suite.toml` (1 trial); every
   read workload emits its expected per-op rows.
4. **Full perf run.** Reads via `configs/read-suite.toml`; write+mixed via
   `scripts/run-write-mixed.sh` (appends to the same report dir). 3 trials;
   reject any cohort with **CV > 0.10**.
5. **Memory.** dhat build:
   `cqlite-perf run --workload read.full_scan --warmup 0 --duration 2s --trials 1`;
   record `custom.mem.live_heap_bytes`.
6. **Scorecard.** `cqlite-perf report --results <jsonl> --goals goals.toml`.
7. **Triage** failures (Reference D).
8. **Promote (tags only).** Re-pin `rev → tag`, reset `CQLITE_VERSION`, record
   baseline, update `CLAUDE.md`, close/cross-link issues.

## Reference B — Correctness matrix (expected row counts)

`probe_point` (basic, 100k): full scan → 100000; `WHERE id='k0000000000000000'`
→ 1; `… '…0001'` → 1; `LIMIT 3` → 3.

`probe_slice` (wide_rows, pk=`p0000` = 1000 ck rows): `WHERE pk='p0000'` → 1000;
`AND ck>=0 AND ck<200` → 200; `AND ck<200` → 200; `AND ck>=800` → 200;
`AND ck BETWEEN 0 AND 199` → 200.  *(Guards cqlite #788.)*

`probe_nonpk` (basic): `WHERE age=0` → 834; `WHERE name='name-0'` → 1;
`WHERE id='k0000000000000000'` → 1.  *(Guards cqlite #586.)*

Read-suite per-op rows (rows ÷ ops in `results.jsonl`): full_scan 100000;
point_lookup 1; clustering_slice 200; type_heavy 100000; wide_partition 100000.

> Asserted by **`cqlite-perf validate`** (#20) — ingests each present corpus,
> checks these counts, and exits non-zero on any mismatch. Absent corpora are
> skipped with a warning.

## Reference C — Measurement protocol (clean room)

- Corpora content-addressed (`datasets/manifests/*.json` + SHA-256);
  `cqlite-perf datasets --check` before a baseline.
- Bare metal, quiesced, on power. **No Docker for measurement** (macOS host mounts
  measure VirtioFS, not the engine). Docker only for corpus gen (cassandra:5.0).
- Fixed `--seed`, `--trials 3`, per-class warmup; reject CV > 0.10.
- Results stamp `host{cpu,cores,os}`; **never compare across host signatures.**

## Reference D — Triage / new-bug workflow

- Correctness fail → minimal `examples/probe_*.rs` that rules out the harness →
  file on `pmcfadin/cqlite` (**issues only, never push code**) → cross-link a
  `pmcfadin/cqlite-perf` tracking issue.
- Perf regression → localize with flamegraph (#6) or dhat (#4 tooling) → attach
  the profile → file upstream.
- Cross-link both directions (upstream # on our issue; harness repro path on theirs).
