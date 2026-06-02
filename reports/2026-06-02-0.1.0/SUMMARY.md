# cqlite-perf — run summary

- **cqlite:** v0.10.0
- **host:** aarch64 (10 cores), macos 25.5.0
- **results:** 13 run(s)

## Throughput by concurrency

| Workload | Codec | Conc | Cache | ops/sec | rows/sec | p50 µs | p99 µs | p99.9 µs | CV |
|---|---|---:|---|---:|---:|---:|---:|---:|---:|
| read.clustering_slice | lz4 | 1 | warm | 7 | 0 | 149631 | 166143 | 175743 | 0.005 |
| read.full_scan | deflate | 1 | warm | 3 | 308410 | 324095 | 425727 | 425727 | 0.050 |
| read.full_scan | lz4 | 1 | warm | 3 | 310760 | 320767 | 337407 | 337407 | 0.006 |
| read.full_scan | none | 1 | warm | 4 | 422990 | 239103 | 269567 | 269567 | 0.029 |
| read.full_scan | snappy | 1 | warm | 3 | 307785 | 324351 | 368895 | 368895 | 0.004 |
| read.full_scan | zstd | 1 | warm | 3 | 307743 | 321791 | 439551 | 439551 | 0.048 |
| read.point_lookup | deflate | 1 | warm | 3 | 0 | 336127 | 375551 | 375551 | 0.029 |
| read.point_lookup | lz4 | 1 | warm | 3 | 0 | 329215 | 391167 | 391167 | 0.026 |
| read.point_lookup | none | 1 | warm | 5 | 0 | 213759 | 234495 | 234495 | 0.013 |
| read.point_lookup | snappy | 1 | warm | 3 | 0 | 350975 | 493823 | 493823 | 0.055 |
| read.point_lookup | zstd | 1 | warm | 3 | 0 | 322303 | 402175 | 402175 | 0.021 |
| read.type_heavy | lz4 | 1 | warm | 2 | 216493 | 463871 | 518143 | 518143 | 0.007 |
| read.wide_partition | lz4 | 1 | warm | 7 | 653099 | 151679 | 232703 | 232703 | 0.019 |

## ⚠ Caveats

The following read workload(s) returned **0 rows** — they executed against a real corpus but over an empty result set, so their latency and throughput numbers are **not meaningful**. Root cause: cqlite v0.10.0 returns no rows for partition-restricted reads (`WHERE pk = ?`); see `docs/feedback-to-cqlite-team.md` §6.
- `read.clustering_slice`
- `read.point_lookup`

## Codec sweep

| Codec | Tier | Size (bytes) | Scan rows/sec | p99 µs |
|---|---|---:|---:|---:|
| deflate | S | 5853184 | 308410 | 425727 |
| lz4 | S | 5865472 | 310760 | 337407 |
| none | S | 35852288 | 422990 | 269567 |
| snappy | S | 6942720 | 307785 | 368895 |
| zstd | S | 5844992 | 307743 | 439551 |
