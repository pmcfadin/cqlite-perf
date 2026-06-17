# cqlite-perf — run summary

- **cqlite:** main@9054734
- **host:** aarch64 (10 cores), macos 25.5.0
- **results:** 17 run(s)

## Throughput by concurrency

| Workload | Codec | Conc | Cache | ops/sec | rows/sec | p50 µs | p99 µs | p99.9 µs | CV |
|---|---|---:|---|---:|---:|---:|---:|---:|---:|
| mixed.open_loop | lz4 | 8 | warm | 99514 | 178254 | 677 | 94143 | 112255 | 0.003 |
| mixed.read_while_write | lz4 | 8 | warm | 426092 | 498764 | 1 | 2 | 12 | 0.001 |
| read.clustering_slice | lz4 | 1 | warm | 6 | 1194 | 164991 | 188799 | 188799 | 0.014 |
| read.full_scan | lz4 | 1 | warm | 3 | 332351 | 300031 | 406527 | 406527 | 0.032 |
| read.point_lookup | lz4 | 1 | warm | 3 | 3 | 302591 | 401151 | 401151 | 0.006 |
| read.type_heavy | lz4 | 1 | warm | 2 | 235940 | 422655 | 455935 | 455935 | 0.005 |
| read.wide_partition | lz4 | 1 | warm | 6 | 587038 | 169599 | 207615 | 207615 | 0.015 |
| write.compaction | lz4 | 1 | warm | 2 | 83170 | 556543 | 663039 | 663039 | 0.008 |
| write.flush | lz4 | 1 | warm | 11 | 260764 | 87743 | 104895 | 152575 | 0.009 |
| write.ingest | lz4 | 1 | warm | 215 | 215 | 4143 | 8591 | 13535 | 0.035 |
| write.ingest | lz4 | 2 | warm | 223 | 223 | 8943 | 15247 | 19519 | 0.049 |
| write.ingest | lz4 | 4 | warm | 255 | 255 | 15007 | 29199 | 36991 | 0.015 |
| write.ingest | lz4 | 8 | warm | 410 | 410 | 17087 | 47231 | 60031 | 0.025 |
| write.ingest_waloff | lz4 | 1 | warm | 254916 | 254916 | 1 | 1 | 3 | 0.005 |
| write.ingest_waloff | lz4 | 2 | warm | 426331 | 426331 | 1 | 2 | 5 | 0.013 |
| write.ingest_waloff | lz4 | 4 | warm | 730228 | 730228 | 1 | 2 | 5 | 0.025 |
| write.ingest_waloff | lz4 | 8 | warm | 922797 | 922797 | 1 | 3 | 6 | 0.050 |

## Write & mixed metrics

| Workload | Conc | Metric | Value |
|---|---:|---|---:|
| mixed.open_loop | 8 | read.ops_per_sec | 43.27 |
| mixed.open_loop | 8 | read.p50_us | 137343 |
| mixed.open_loop | 8 | read.p999_us | 162175 |
| mixed.open_loop | 8 | read.p99_us | 157695 |
| mixed.open_loop | 8 | write.ops_per_sec | 99470.42 |
| mixed.open_loop | 8 | write.p50_us | 675 |
| mixed.open_loop | 8 | write.p999_us | 102591 |
| mixed.open_loop | 8 | write.p99_us | 93247 |
| mixed.open_loop | 8 | write.target_ops_per_sec | 100000 |
| mixed.read_while_write | 8 | read.ops_per_sec | 38.76 |
| mixed.read_while_write | 8 | read.p50_us | 152575 |
| mixed.read_while_write | 8 | read.p999_us | 257023 |
| mixed.read_while_write | 8 | read.p99_us | 239871 |
| mixed.read_while_write | 8 | write.ops_per_sec | 426052.45 |
| mixed.read_while_write | 8 | write.p50_us | 1 |
| mixed.read_while_write | 8 | write.p999_us | 12 |
| mixed.read_while_write | 8 | write.p99_us | 2 |
| write.compaction | 1 | compaction.cycles | 27 |
| write.compaction | 1 | compaction.merge_mb_per_sec | 40.04 |
| write.compaction | 1 | compaction.output_mb_mean | 13.76 |
| write.compaction | 1 | compaction.read_amp_after | 1 |
| write.compaction | 1 | compaction.read_amp_before | 4 |
| write.compaction | 1 | compaction.wall_ms_mean | 343.65 |
| write.flush | 1 | flush.count | 114 |
| write.flush | 1 | flush.latency_ms_mean | 75.40 |
| write.flush | 1 | flush.mb_per_sec | 106.10 |
| write.flush | 1 | flush.ondisk_mb_mean | 6.88 |
