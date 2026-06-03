# cqlite-perf — run summary

- **cqlite:** v0.10.0
- **host:** aarch64 (10 cores), macos 25.5.0
- **results:** 25 run(s)

## Throughput by concurrency

| Workload | Codec | Conc | Cache | ops/sec | rows/sec | p50 µs | p99 µs | p99.9 µs | CV |
|---|---|---:|---|---:|---:|---:|---:|---:|---:|
| mixed.open_loop | lz4 | 8 | warm | 99276 | 246795 | 744 | 89919 | 140927 | 0.006 |
| mixed.read_while_write | lz4 | 8 | warm | 464558 | 598377 | 1 | 2 | 10 | 0.006 |
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
| write.compaction | lz4 | 1 | warm | 2 | 97280 | 460543 | 645631 | 645631 | 0.002 |
| write.flush | lz4 | 1 | warm | 13 | 309087 | 73343 | 95167 | 104063 | 0.008 |
| write.ingest | lz4 | 1 | warm | 247 | 247 | 3997 | 7263 | 12743 | 0.013 |
| write.ingest | lz4 | 2 | warm | 244 | 244 | 7995 | 12951 | 16991 | 0.038 |
| write.ingest | lz4 | 4 | warm | 299 | 299 | 13903 | 21967 | 25311 | 0.020 |
| write.ingest | lz4 | 8 | warm | 482 | 482 | 15103 | 30879 | 43103 | 0.021 |
| write.ingest_waloff | lz4 | 1 | warm | 273412 | 273412 | 1 | 1 | 4 | 0.014 |
| write.ingest_waloff | lz4 | 2 | warm | 478411 | 478411 | 1 | 2 | 5 | 0.020 |
| write.ingest_waloff | lz4 | 4 | warm | 534884 | 534884 | 1 | 2 | 6 | 0.338 |
| write.ingest_waloff | lz4 | 8 | warm | 993317 | 993317 | 1 | 2 | 6 | 0.019 |

## Write & mixed metrics

| Workload | Conc | Metric | Value |
|---|---:|---|---:|
| mixed.open_loop | 8 | read.ops_per_sec | 34.61 |
| mixed.open_loop | 8 | read.p50_us | 167551 |
| mixed.open_loop | 8 | read.p999_us | 393215 |
| mixed.open_loop | 8 | read.p99_us | 349951 |
| mixed.open_loop | 8 | write.ops_per_sec | 99241.01 |
| mixed.open_loop | 8 | write.p50_us | 740 |
| mixed.open_loop | 8 | write.p999_us | 98239 |
| mixed.open_loop | 8 | write.p99_us | 87935 |
| mixed.open_loop | 8 | write.target_ops_per_sec | 100000 |
| mixed.read_while_write | 8 | read.ops_per_sec | 30.86 |
| mixed.read_while_write | 8 | read.p50_us | 194943 |
| mixed.read_while_write | 8 | read.p999_us | 379135 |
| mixed.read_while_write | 8 | read.p99_us | 350463 |
| mixed.read_while_write | 8 | write.ops_per_sec | 464527.43 |
| mixed.read_while_write | 8 | write.p50_us | 1 |
| mixed.read_while_write | 8 | write.p999_us | 9 |
| mixed.read_while_write | 8 | write.p99_us | 2 |
| write.compaction | 1 | compaction.cycles | 32 |
| write.compaction | 1 | compaction.merge_mb_per_sec | 48.95 |
| write.compaction | 1 | compaction.output_mb_mean | 13.77 |
| write.compaction | 1 | compaction.read_amp_after | 1 |
| write.compaction | 1 | compaction.read_amp_before | 4 |
| write.compaction | 1 | compaction.wall_ms_mean | 281.30 |
| write.flush | 1 | flush.count | 134 |
| write.flush | 1 | flush.latency_ms_mean | 63.78 |
| write.flush | 1 | flush.mb_per_sec | 125.43 |
| write.flush | 1 | flush.ondisk_mb_mean | 6.89 |

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
