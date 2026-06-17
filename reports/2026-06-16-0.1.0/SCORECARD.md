# cqlite-perf — goals scorecard

cqlite **main@9054734** · no baseline · aarch64 (10 cores)

| Goal | Metric | Target | Actual | vs base | Status | Enforced |
|------|--------|-------:|-------:|--------:|--------|:--------:|
| basic scan decode throughput (S tier) | throughput.rows_per_sec | >= 1,000,000 | 332,351 | — | ❌ MISSED | no |
| point lookup p99 latency | latency_us.p99 | <= 500 | 401,151 | — | ❌ MISSED | no |
| streaming scan live heap bounded (S tier) | custom.mem.live_heap_bytes | <= 268,435,456 | — | — | — NO-DATA | no |
| streaming scan peak RSS (informational) | resource.peak_rss_bytes | <= 3,221,225,472 | 1,707,982,848 | — | ✅ MET | no |
| write ingest throughput (WAL-on) | throughput.ops_per_sec | >= 250 | 215 | — | ❌ MISSED | no |
| write ingest throughput (WAL-off) | throughput.ops_per_sec | >= 200,000 | 254,916 | — | ✅ MET | no |
| memtable flush throughput | custom.flush.mb_per_sec | >= 100 | 106 | — | ✅ MET | no |
| memtable flush latency | custom.flush.latency_ms_mean | <= 100 | 75 | — | ✅ MET | no |
| compaction read-amp reduction | custom.compaction.read_amp_after | <= 1 | 1 | — | ✅ MET | no |
| compaction cycle wall-time | custom.compaction.wall_ms_mean | <= 500 | 344 | — | ✅ MET | no |
| read p99 under write load (read_while_write) | custom.read.p99_us | <= 500,000 | 239,871 | — | ✅ MET | no |
| open-loop writers keep up with target rate | custom.write.ops_per_sec | >= 90,000 | 99,470 | — | ✅ MET | no |
| read p99 under open-loop write load | custom.read.p99_us | <= 500,000 | 157,695 | — | ✅ MET | no |
| scan throughput regression budget | throughput.rows_per_sec | ≤10% regress | 332,351 | — | — NO-DATA | no |

14 goal(s), 3 failed (0 enforced).
