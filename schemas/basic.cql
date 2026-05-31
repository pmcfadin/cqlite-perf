-- Basic-types perf schema (SPEC §8). A single-partition-key table with a
-- handful of scalar columns, sized for the read suite's point/scan workloads.
-- Generated into keyspace `perf`, table `basic`.
CREATE TABLE perf.basic (
    id        text PRIMARY KEY,
    name      text,
    email     text,
    age       int,
    payload   text
);
