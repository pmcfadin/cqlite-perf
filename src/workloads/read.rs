//! Read suite (SPEC §6). Lands in M1 atop the Cassandra-generated canonical
//! corpus (SPEC §8.1): point_lookup, clustering_slice, full_scan,
//! wide_partition, type_heavy.
//!
//! M0 is intentionally empty — a read workload needs a pre-built SSTable
//! corpus, which is M1's deliverable. The M0 loop is proven by `write.ingest`,
//! which needs no external data.
