# cqlite-perf — goals scorecard

cqlite **v0.12.0** · no baseline · aarch64 (10 cores)

| Goal | Metric | Target | Actual | vs base | Status | Enforced |
|------|--------|-------:|-------:|--------:|--------|:--------:|
| basic scan decode throughput (S tier) | throughput.rows_per_sec | >= 300,000 | 469,807 | — | ✅ MET | no |
| point lookup p99 latency | latency_us.p99 | <= 450,000 | 300,799 | — | ✅ MET | no |
| streaming scan live heap bounded (S tier) | custom.mem.live_heap_bytes | <= 268,435,456 | — | — | — NO-DATA | no |
| streaming scan peak RSS (informational) | resource.peak_rss_bytes | <= 3,221,225,472 | 892,207,104 | — | ✅ MET | no |
| write ingest throughput (WAL-on) | throughput.ops_per_sec | >= 200 | 229 | — | ✅ MET | no |
| write ingest throughput (WAL-off) | throughput.ops_per_sec | >= 230,000 | 243,999 | — | ✅ MET | no |
| memtable flush throughput | custom.flush.mb_per_sec | >= 95 | 100 | — | ✅ MET | no |
| memtable flush latency | custom.flush.latency_ms_mean | <= 85 | 80 | — | ✅ MET | no |
| compaction read-amp reduction | custom.compaction.read_amp_after | <= 1 | 1 | — | ✅ MET | no |
| compaction cycle wall-time | custom.compaction.wall_ms_mean | <= 400 | 219 | — | ✅ MET | no |
| read p99 under write load (read_while_write) | custom.read.p99_us | <= 275,000 | 355,839 | — | ❌ MISSED | no |
| open-loop writers keep up with target rate | custom.write.ops_per_sec | >= 90,000 | 98,583 | — | ✅ MET | no |
| read p99 under open-loop write load | custom.read.p99_us | <= 185,000 | 365,055 | — | ❌ MISSED | no |
| scan throughput regression budget | throughput.rows_per_sec | ≤10% regress | 469,807 | — | — NO-DATA | no |
| scan throughput regression vs baseline | throughput.rows_per_sec | ≤30% regress | 469,807 | — | — NO-DATA | yes |
| point lookup p99 regression vs baseline | latency_us.p99 | ≤30% regress | 300,799 | — | — NO-DATA | yes |

16 goal(s), 2 failed (0 enforced).
