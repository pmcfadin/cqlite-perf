-- Wide-partition perf schema (SPEC §6 read.wide_partition + read.clustering_slice).
-- Composite PK with few partitions and many clustering rows each, so a single
-- partition is genuinely wide. Generated into keyspace `perf`, table `wide_rows`;
-- gen-corpus.sh loads 100 partitions × (rows/100) clustering rows.
CREATE TABLE perf.wide_rows (
    pk    text,
    ck    int,
    val   text,
    PRIMARY KEY (pk, ck)
);
