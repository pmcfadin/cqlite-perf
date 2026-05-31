# cqlite-perf — run summary

- **cqlite:** v0.9.2
- **host:** aarch64 (10 cores), macos 25.5.0
- **results:** 2 run(s)

## Throughput by concurrency

| Workload | Conc | Cache | ops/sec | rows/sec | p50 µs | p99 µs | p99.9 µs | CV |
|---|---:|---|---:|---:|---:|---:|---:|---:|
| write.ingest | 1 | warm | 299 | 299 | 3047 | 7155 | 12991 | 0.000 |
| write.ingest | 2 | warm | 267 | 267 | 7063 | 12631 | 17183 | 0.000 |
