# cqlite-perf — goals scorecard

cqlite **v0.10.0** · no baseline · aarch64 (10 cores)

| Goal | Metric | Target | Actual | vs base | Status | Enforced |
|------|--------|-------:|-------:|--------:|--------|:--------:|
| basic scan decode throughput (S tier) | throughput.rows_per_sec | >= 1,000,000 | 422,990 | — | ❌ MISSED | no |
| point lookup p99 latency | latency_us.p99 | <= 500 | 234,495 | — | ❌ MISSED | no |
| streaming scan stays memory-bounded | resource.peak_rss_bytes | <= 134,217,728 | 683,966,464 | — | ❌ MISSED | no |
| write ingest throughput (WAL-on) | throughput.ops_per_sec | >= 250 | 247 | — | ❌ MISSED | no |
| write ingest throughput (WAL-off) | throughput.ops_per_sec | >= 200,000 | 273,412 | — | ✅ MET | no |
| memtable flush throughput | custom.flush.mb_per_sec | >= 100 | 125 | — | ✅ MET | no |
| memtable flush latency | custom.flush.latency_ms_mean | <= 100 | 64 | — | ✅ MET | no |
| compaction read-amp reduction | custom.compaction.read_amp_after | <= 1 | 1 | — | ✅ MET | no |
| compaction cycle wall-time | custom.compaction.wall_ms_mean | <= 500 | 281 | — | ✅ MET | no |
| read p99 under write load (read_while_write) | custom.read.p99_us | <= 500,000 | 350,463 | — | ✅ MET | no |
| open-loop writers keep up with target rate | custom.write.ops_per_sec | >= 90,000 | 99,241 | — | ✅ MET | no |
| read p99 under open-loop write load | custom.read.p99_us | <= 500,000 | 349,951 | — | ✅ MET | no |
| scan throughput regression budget | throughput.rows_per_sec | ≤10% regress | 422,990 | — | — NO-DATA | no |

13 goal(s), 4 failed (0 enforced).
