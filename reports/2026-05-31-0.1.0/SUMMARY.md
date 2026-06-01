# cqlite-perf — run summary

- **cqlite:** v0.9.2
- **host:** aarch64 (10 cores), macos 25.5.0
- **results:** 13 run(s)

## Throughput by concurrency

| Workload | Codec | Conc | Cache | ops/sec | rows/sec | p50 µs | p99 µs | p99.9 µs | CV |
|---|---|---:|---|---:|---:|---:|---:|---:|---:|
| read.clustering_slice | lz4 | 1 | warm | 5 | 0 | 194815 | 301311 | 301311 | 0.035 |
| read.full_scan | deflate | 1 | warm | 2 | 217020 | 460031 | 486399 | 486399 | 0.006 |
| read.full_scan | lz4 | 1 | warm | 2 | 218440 | 455679 | 478975 | 478975 | 0.003 |
| read.full_scan | none | 1 | warm | 4 | 425162 | 236287 | 244991 | 244991 | 0.009 |
| read.full_scan | snappy | 1 | warm | 2 | 216766 | 457983 | 474623 | 474623 | 0.007 |
| read.full_scan | zstd | 1 | warm | 2 | 215738 | 464639 | 510207 | 510207 | 0.005 |
| read.point_lookup | deflate | 1 | warm | 2 | 0 | 398591 | 525823 | 525823 | 0.011 |
| read.point_lookup | lz4 | 1 | warm | 2 | 0 | 438015 | 507135 | 507135 | 0.031 |
| read.point_lookup | none | 1 | warm | 5 | 0 | 208255 | 240127 | 240127 | 0.005 |
| read.point_lookup | snappy | 1 | warm | 2 | 0 | 420095 | 441599 | 441599 | 0.028 |
| read.point_lookup | zstd | 1 | warm | 2 | 0 | 426751 | 449535 | 449535 | 0.035 |
| read.type_heavy | lz4 | 1 | warm | 2 | 220045 | 451583 | 501759 | 501759 | 0.014 |
| read.wide_partition | lz4 | 1 | warm | 7 | 666797 | 149631 | 161023 | 165375 | 0.012 |

## ⚠ Caveats

The following read workload(s) returned **0 rows** — they executed against a real corpus but over an empty result set, so their latency and throughput numbers are **not meaningful**. Root cause: cqlite v0.9.2 returns no rows for partition-restricted reads (`WHERE pk = ?`); see `docs/feedback-to-cqlite-team.md` §6.
- `read.clustering_slice`
- `read.point_lookup`

## Codec sweep

| Codec | Tier | Size (bytes) | Scan rows/sec | p99 µs |
|---|---|---:|---:|---:|
| deflate | S | 5853184 | 217020 | 486399 |
| lz4 | S | 5865472 | 218440 | 478975 |
| none | S | 35852288 | 425162 | 244991 |
| snappy | S | 6942720 | 216766 | 474623 |
| zstd | S | 5844992 | 215738 | 510207 |
