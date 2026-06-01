# cqlite-perf — goals scorecard

cqlite **v0.9.2** · no baseline · aarch64 (10 cores)

| Goal | Metric | Target | Actual | vs base | Status | Enforced |
|------|--------|-------:|-------:|--------:|--------|:--------:|
| basic scan decode throughput (S tier) | throughput.rows_per_sec | >= 1,000,000 | 425,162 | — | ❌ MISSED | no |
| point lookup p99 latency | latency_us.p99 | <= 500 | 240,127 | — | ❌ MISSED | no |
| streaming scan stays memory-bounded | resource.peak_rss_bytes | <= 134,217,728 | 735,281,152 | — | ❌ MISSED | no |
| write ingest throughput (WAL-on) | throughput.ops_per_sec | >= 250 | — | — | — NO-DATA | no |
| scan throughput regression budget | throughput.rows_per_sec | ≤10% regress | 425,162 | — | — NO-DATA | no |

5 goal(s), 3 failed (0 enforced).
