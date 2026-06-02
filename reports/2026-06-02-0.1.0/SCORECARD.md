# cqlite-perf — goals scorecard

cqlite **v0.10.0** · no baseline · aarch64 (10 cores)

| Goal | Metric | Target | Actual | vs base | Status | Enforced |
|------|--------|-------:|-------:|--------:|--------|:--------:|
| basic scan decode throughput (S tier) | throughput.rows_per_sec | >= 1,000,000 | 422,990 | — | ❌ MISSED | no |
| point lookup p99 latency | latency_us.p99 | <= 500 | 234,495 | — | ❌ MISSED | no |
| streaming scan stays memory-bounded | resource.peak_rss_bytes | <= 134,217,728 | 683,966,464 | — | ❌ MISSED | no |
| write ingest throughput (WAL-on) | throughput.ops_per_sec | >= 250 | — | — | — NO-DATA | no |
| scan throughput regression budget | throughput.rows_per_sec | ≤10% regress | 422,990 | — | — NO-DATA | no |

5 goal(s), 3 failed (0 enforced).
