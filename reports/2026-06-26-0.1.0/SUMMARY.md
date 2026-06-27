# cqlite-perf — run summary

- **cqlite:** v0.12.0
- **host:** aarch64 (10 cores), macos 25.5.0
- **results:** 17 run(s)

## Throughput by concurrency

| Workload | Codec | Conc | Cache | ops/sec | rows/sec | p50 µs | p99 µs | p99.9 µs | CV |
|---|---|---:|---|---:|---:|---:|---:|---:|---:|
| mixed.open_loop | lz4 | 8 | warm | 98606 | 2469121 | 854 | 144127 | 215551 | 0.002 |
| mixed.read_while_write | lz4 | 8 | warm | 285192 | 2627346 | 1 | 3 | 134 | 0.037 |
| read.clustering_slice | lz4 | 1 | warm | 12 | 2358 | 85055 | 109503 | 116607 | 0.020 |
| read.full_scan | lz4 | 1 | warm | 5 | 469807 | 209535 | 244095 | 244095 | 0.054 |
| read.point_lookup | lz4 | 1 | warm | 6 | 6 | 172927 | 300799 | 300799 | 0.095 |
| read.type_heavy | lz4 | 1 | warm | 2 | 244670 | 400639 | 433663 | 433663 | 0.084 |
| read.wide_partition | lz4 | 1 | warm | 7 | 691117 | 145791 | 175871 | 254207 | 0.051 |
| write.compaction | lz4 | 1 | warm | 2 | 108271 | 428287 | 485375 | 485375 | 0.004 |
| write.flush | lz4 | 1 | warm | 11 | 251399 | 91327 | 109183 | 137087 | 0.007 |
| write.ingest | lz4 | 1 | warm | 229 | 229 | 4047 | 16591 | 29199 | 0.089 |
| write.ingest | lz4 | 2 | warm | 226 | 226 | 8163 | 16095 | 22031 | 0.023 |
| write.ingest | lz4 | 4 | warm | 283 | 283 | 14015 | 23055 | 26991 | 0.019 |
| write.ingest | lz4 | 8 | warm | 472 | 472 | 16031 | 37119 | 52031 | 0.063 |
| write.ingest_waloff | lz4 | 1 | warm | 243999 | 243999 | 1 | 1 | 4 | 0.026 |
| write.ingest_waloff | lz4 | 2 | warm | 420789 | 420789 | 1 | 2 | 5 | 0.013 |
| write.ingest_waloff | lz4 | 4 | warm | 719539 | 719539 | 1 | 2 | 5 | 0.021 |
| write.ingest_waloff | lz4 | 8 | warm | 930820 | 930820 | 1 | 3 | 8 | 0.043 |

## Write & mixed metrics

| Workload | Conc | Metric | Value |
|---|---:|---|---:|
| mixed.open_loop | 8 | read.ops_per_sec | 23.70 |
| mixed.open_loop | 8 | read.p50_us | 242047 |
| mixed.open_loop | 8 | read.p999_us | 396287 |
| mixed.open_loop | 8 | read.p99_us | 365055 |
| mixed.open_loop | 8 | write.ops_per_sec | 98582.79 |
| mixed.open_loop | 8 | write.p50_us | 828 |
| mixed.open_loop | 8 | write.p999_us | 216191 |
| mixed.open_loop | 8 | write.p99_us | 150911 |
| mixed.open_loop | 8 | write.target_ops_per_sec | 100000 |
| mixed.read_while_write | 8 | read.ops_per_sec | 23.42 |
| mixed.read_while_write | 8 | read.p50_us | 250239 |
| mixed.read_while_write | 8 | read.p999_us | 392447 |
| mixed.read_while_write | 8 | read.p99_us | 355839 |
| mixed.read_while_write | 8 | write.ops_per_sec | 285168.65 |
| mixed.read_while_write | 8 | write.p50_us | 1 |
| mixed.read_while_write | 8 | write.p999_us | 124 |
| mixed.read_while_write | 8 | write.p99_us | 3 |
| write.compaction | 1 | compaction.cycles | 35 |
| write.compaction | 1 | compaction.merge_mb_per_sec | 62.94 |
| write.compaction | 1 | compaction.output_mb_mean | 13.76 |
| write.compaction | 1 | compaction.read_amp_after | 1 |
| write.compaction | 1 | compaction.read_amp_before | 4 |
| write.compaction | 1 | compaction.wall_ms_mean | 218.64 |
| write.flush | 1 | flush.count | 109 |
| write.flush | 1 | flush.latency_ms_mean | 79.91 |
| write.flush | 1 | flush.mb_per_sec | 100.11 |
| write.flush | 1 | flush.ondisk_mb_mean | 6.88 |
