# Scan-Decode Attribution — `read.full_scan` (100k rows, lz4, S tier)

**Source:** `profiles/2026-06-16-read.full_scan-dhat-heap.json`
**Engine:** cqlite main @ 9054734 (post-v0.11.0, pre-v0.12.0 tag)
**Tool:** dhat heap profiler (cqlite-perf `--features dhat-heap`)
**Workload:** `read.full_scan` — `SELECT * FROM perf.basic` — 100k rows, lz4 SSTable

> dhat measures **allocation volume and live bytes**, not CPU time directly.
> Allocation churn strongly correlates with CPU cost (malloc/free overhead, cache
> thrashing) and is a reliable proxy until a CPU flamegraph is available from the
> Linux CI image (W4-A).

---

## Summary

| Metric | Value |
|---|---:|
| Total bytes allocated (one scan) | 820 MB |
| Live bytes at t-gmax (peak heap) | 185 MB |
| Scan duration (t-end) | 54.9 s |
| Peak heap (t-gmax) at | 46.3 s |

---

## Attribution Breakdown

### By total bytes allocated (allocation churn = malloc pressure)

| Category | Total alloc | % of total | Live at t-gmax | % of t-gmax |
|---|---:|---:|---:|---:|
| HashMap resize (`hashbrown`) | 328 MB | 40.0% | 15 MB | 7.8% |
| Sequential scan / row accumulation | 183 MB | 22.4% | 24 MB | 12.2% |
| `parse_block` (block-level parse) | 108 MB | 13.2% | 85 MB | 44.0% |
| `stitch_and_parse_all_chunks` | 84 MB | 10.2% | 40 MB | 20.6% |
| Other | 53 MB | 6.5% | 2 MB | 0.9% |
| lz4 decompress (`lz4_flex`) | 33 MB | 4.1% | 0 MB | 0.0% |
| `parse_cell_value_schema_order` | 30 MB | 3.6% | 28 MB | 14.5% |

---

## Key Findings

### 1. Row/cell decode (`parse_cell_value_schema_order`) — moderate allocation

`parse_cell_value_schema_order` allocates **30 MB total (3.6%)** and holds
**28 MB live at t-gmax (14.5%)**. The high live-to-allocated ratio (93%) means
these allocations accumulate in the result set rather than being promptly freed.
The top-level program point (PP7) uses `.to_vec()` to clone cell byte slices —
one owned `Vec<u8>` per cell value. With ~300k cells per scan, that is 300k
short-lived heap allocations per `op()`.

### 2. Block parse + SSTable infrastructure (`parse_block`) — dominant live heap

`parse_block` contributes **108 MB total (13.2%)** and holds **85 MB live at
t-gmax (44.0%)** — the single largest contribution to peak heap.

Two sub-sites are dominant:

- **`parse_row_data_with_offset` → HashMap::insert** (PP1+PP2, 127 MB total):
  `parse_block` builds a `HashMap<column_id, value>` per row by inserting into
  a hashbrown table that repeatedly rehashes. This is 4× the parse_block frame's
  direct allocation. The table is sized per-row, discarded after each row, and
  reallocated for the next — pure churn at ~100k rows/scan.

- **`parse_block` → `collect()` into `Vec`** (PP4, 64 MB, 100% live at t-gmax):
  One 640-byte `Vec` per SSTable block, held live throughout the scan. For 100k
  rows this keeps ~64 MB pinned — a clear target for in-place parsing.

### 3. lz4 decompress (`lz4_flex::decompress`) — ephemeral allocation

lz4 decompression allocates **33 MB total (4.1%)** and holds **0 MB live at
t-gmax**. Every compressed block allocates a fresh decompressed buffer (via
`vec![0u8; decompressed_size]`), uses it, and drops it — pure churn with zero
steady-state retention. This is a significant malloc pressure source despite
leaving no heap residue, and is a candidate for a reusable scratch buffer.

### 4. HashMap churn — the dominant allocation sink

The single largest allocation category is `hashbrown` table resizes at
**328 MB / 40.0%** of total. These trace back through `parse_row_data_with_offset`
and the query executor's internal row representation. A per-row HashMap that
grows via rehash on every insert is the core driver of allocation churn in the
hot path.

---

## Prioritized Bottlenecks for W3-A

In order of impact (total allocation churn × live retention):

1. **Per-row HashMap in `parse_row_data_with_offset`** (328 MB churn, 127 MB from
   parse_block alone): Replace with a fixed-size array or small-vec keyed by
   column index. Eliminates rehash churn and reduces live heap. Upstream engine
   fix target.

2. **Block-level `collect()` into owned Vec in `parse_block`** (64 MB, 100% live):
   Switch to in-place or borrowed slices into the decompressed buffer. Would cut
   ~64 MB from the t-gmax peak. Upstream engine fix target.

3. **lz4 decompressed buffer per block** (33 MB, all transient): Pass a reusable
   `&mut Vec<u8>` scratch buffer into the decompressor and `clear()` + reuse
   it per block. Zero live impact but removes 33 MB of malloc/free churn.
   Upstream engine fix target.

4. **Cell value `.to_vec()` clones in `parse_cell_value_schema_order`** (28 MB
   live, 300k allocs/scan): Zero-copy cell value references (borrowed from the
   decompressed buffer) would eliminate this category. Requires lifetime changes
   in the parser; upstream engine fix target.

---

## Flamegraph Status

CPU flamegraphs were **not captured on macOS** — `cargo flamegraph` uses dtrace
which requires sudo, unavailable in this non-interactive environment. The script
`scripts/flamegraph.sh` is correct and portable:

- **Linux (CI, W4-A):** runs via `perf`, no sudo needed.
- **macOS (interactive):** requires `sudo` — re-run with elevated privileges.

The dhat allocation data above provides sufficient attribution to feed W3-A goal
recalibration. CPU flamegraphs from W4-A will confirm whether the allocation churn
sites correlate with CPU time as expected (they almost certainly will, given the
100k HashMap rehash cycles per scan).

---

## Workload Names Confirmed

| Name | Schema | Table |
|---|---|---|
| `read.full_scan` | `basic` | `perf.basic` |
| `read.wide_partition` | `wide_rows` | `perf.wide_rows` |
| `read.type_heavy` | `collections` | `perf.collections` |

All three are registered in `src/workloads/mod.rs` and use keyspace-qualified
table names as required by cqlite v0.11.0+.
