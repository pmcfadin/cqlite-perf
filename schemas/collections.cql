-- Collections perf schema (SPEC §6 read.type_heavy). A single-partition-key
-- table carrying set/list/map collection columns so the scan path exercises
-- collection deserialization cost vs. raw I/O. Generated into keyspace `perf`,
-- table `collections`; the codec is applied via ALTER in gen-corpus.sh.
CREATE TABLE perf.collections (
    id        text PRIMARY KEY,
    tags      set<text>,
    scores    list<int>,
    props     map<text, text>,
    payload   text
);
