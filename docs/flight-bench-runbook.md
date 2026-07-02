# Distributed Flight/Trino benchmarking runbook (issue #36, epic #30 T5)

Status: **scaffold — commands drafted, not yet executed.** This is the
ready-to-run procedure for the easy-db-lab spike; it needs a lab AWS account
(`easy-db-lab setup-profile`) before any step below can actually run. See
issue #36 for the full design rationale; this doc is the executable half.

## Why this exists

`cqlite-flight` is CQLite's distributed data plane: an Arrow Flight server
co-located with each Cassandra node, k-way-merging that node's SSTables on
read and streaming Arrow batches — the backend of the Trino connector. None
of its performance is measured anywhere today (single-node harness workloads
don't exercise it). This spike measures it on a real multi-node cluster.

## Prerequisites

- A **lab-dedicated** AWS account (easy-db-lab provisions/destroys real
  infrastructure — never point it at a production account):
  ```bash
  brew tap rustyrazorblade/rustyrazorblade && brew install easy-db-lab
  easy-db-lab show-iam-policies   # print the policies to attach
  easy-db-lab setup-profile        # one-time: credentials, keypair, IAM role
  ```
- `ghcr.io/pmcfadin/cqlite-flight` is a public multi-arch image — no
  `docker login` needed to pull it.
- A Trino distribution with the `in.mcfad:cqlite-trino` connector jar
  available (check the connector's own release page for the exact coordinate
  and Trino version compatibility at benchmark time).

## 1. Provision the cluster

```bash
easy-db-lab up \
  --nodes 3 --database cassandra --version 5.0 \
  --instance-type <NVMe-backed class, e.g. i4i.xlarge — confirm current pricing/availability first> \
  --stress-node
```

This provisions 3 Cassandra 5.0 nodes + 1 stress/coordinator node, K3s across
all of them, and pre-wires Prometheus + Grafana. Confirm cluster health and
grab node IPs:

```bash
easy-db-lab status
easy-db-lab ssh <node-1>   # spot-check: cassandra service up, nodetool status
```

## 2. Load data

From the stress/coordinator node, using the pre-installed
`cassandra-easy-stress` (mirror the harness's own schemas so findings
translate — `basic` ~ KeyValue, `wide_rows` ~ a wide-partition/time-series
profile):

```bash
cassandra-easy-stress run KeyValue \
  --host <any-node-ip> --replication 3 \
  --partitions 5000000 --duration 20m
cassandra-easy-stress run <wide-partition-profile> \
  --host <any-node-ip> --replication 3 \
  --partitions 50000 --duration 20m
```

Flush and snapshot on **every** node (Flight reads flushed SSTables — memtable
rows are invisible by design; the snapshot gives a consistent file set while
Cassandra keeps compacting underneath):

```bash
for n in <node-1> <node-2> <node-3>; do
  easy-db-lab ssh "$n" -- "nodetool flush && nodetool snapshot -t bench-run-1 <keyspace>"
done
```

## 3. Deploy cqlite-flight (one per Cassandra node, K3s)

Per node, a `DaemonSet` (one pod per node, hostPath-mounting that node's own
data dir — do NOT use a shared/networked volume, each pod must see its local
node's SSTables only):

```yaml
# k3s/cqlite-flight-daemonset.yaml
apiVersion: apps/v1
kind: DaemonSet
metadata:
  name: cqlite-flight
spec:
  selector:
    matchLabels: {app: cqlite-flight}
  template:
    metadata:
      labels: {app: cqlite-flight}
    spec:
      hostNetwork: true   # simplest way to expose :8815 per-node-IP for Trino's per-replica addressing
      containers:
        - name: cqlite-flight
          image: ghcr.io/pmcfadin/cqlite-flight:latest
          args: ["--data-dir", "/var/lib/cassandra/data", "--listen", "0.0.0.0:8815"]
          env: [{name: RUST_LOG, value: "info"}]
          ports: [{containerPort: 8815, hostPort: 8815}]
          volumeMounts:
            - {name: cassandra-data, mountPath: /var/lib/cassandra, readOnly: true}
      volumes:
        - name: cassandra-data
          hostPath: {path: /var/lib/cassandra}
```

```bash
kubectl apply -f k3s/cqlite-flight-daemonset.yaml
kubectl get pods -o wide   # confirm one cqlite-flight pod per Cassandra node
```

Sanity-check one endpoint directly before wiring up Trino — a raw ticket
against a single node (ticket schema per `cqlite-flight/README.md`):

```bash
python3 -c "
import pyarrow.flight as fl, json
c = fl.FlightClient('grpc://<node-1-ip>:8815')
ticket = fl.Ticket(json.dumps({
    'version': 1, 'keyspace': '<ks>', 'table': '<tbl>',
    'ddl': '<CREATE TABLE ... from cqlsh DESCRIBE TABLE>',
}).encode())
reader = c.do_get(ticket)
print(reader.read_all().num_rows)
"
```

## 4. Deploy Trino (K3s, two catalogs on the same cluster)

```yaml
# k3s/trino-catalogs/cqlite.properties
connector.name=cqlite
cqlite.nodes=<node-1-ip>:8815,<node-2-ip>:8815,<node-3-ip>:8815
# (exact property names/coordinate per the in.mcfad:cqlite-trino connector's
# own docs at benchmark time — confirm before running)
```
```yaml
# k3s/trino-catalogs/cassandra.properties
connector.name=cassandra
cassandra.contact-points=<node-1-ip>,<node-2-ip>,<node-3-ip>
cassandra.load-policy.dc-aware.local-dc=<dc-name>
```

Deploy Trino as a K3s Deployment mounting both catalog files, confirm both
catalogs are visible:

```bash
kubectl exec -it deploy/trino -- trino --execute "SHOW CATALOGS"
```

## 5. Correctness gate — BEFORE any timing (standing rule)

```sql
-- via the cqlite catalog
SELECT count(*) FROM cqlite.<ks>.<tbl>;
SELECT sum(<numeric-col>) FROM cqlite.<ks>.<tbl>;   -- checksum-style aggregate
-- vs the cassandra catalog (or cqlsh directly) on the SAME data
SELECT count(*) FROM cassandra.<ks>.<tbl>;
SELECT sum(<numeric-col>) FROM cassandra.<ks>.<tbl>;
```
Row counts and aggregates must match exactly. **Do not proceed to timing
until this passes** — a fast wrong answer is not a result.

## 6. Measurement axes

Run each with `time` wrapped around the Trino CLI call (or `EXPLAIN ANALYZE`
for a query-plan-level timing breakdown); capture Grafana screenshots for the
resource-impact axes (5) since those need to be *observed during* the query,
not just timed.

### Axis 1 — single-endpoint DoGet throughput
Direct pyarrow `do_get` against one node (bypass Trino), cold (drop page
cache via `echo 1 > /proc/sys/vm/drop_caches` on the node first, needs root)
vs warm. Record rows/s and Arrow MB/s (`reader.read_all().nbytes`).

### Axis 2 — merge cost vs SSTable count
Force different SSTable counts per table via flush cadence during load (flush
every N inserts for a "many small SSTables" case) vs a single
`nodetool compact` pass (one big SSTable). Repeat axis 1 at each point.

### Axis 3 — token-range parallelism
```sql
SELECT count(*) FROM cqlite.<ks>.<tbl>;  -- full scan via Trino, all 3 nodes in parallel
```
Compare aggregate rows/s against axis-1's single-node number × 3 — how close
to linear?

### Axis 4 — Trino: cqlite vs stock Cassandra connector (the headline)
Same queries, same cluster, same data, both catalogs:
```sql
SELECT count(*) FROM {catalog}.<ks>.<tbl>;
SELECT * FROM {catalog}.<ks>.<tbl> WHERE <predicate>;         -- filtered scan
SELECT <col1>, <col2> FROM {catalog}.<ks>.<tbl>;              -- narrow projection
SELECT * FROM {catalog}.<ks>.<tbl> WHERE <pk-col> = <value>;  -- partition lookup
```
Record latency + rows/s for both `{catalog}` = `cqlite` and `cassandra`.

### Axis 5 — co-location impact (the cluster-level #1143 question)
```bash
cassandra-easy-stress run KeyValue --host <node-1-ip> --duration 5m   # baseline read/write p99
# then repeat WHILE concurrently running a Flight full-scan against the same node
```
Watch the pre-wired Grafana dashboards (CPU, disk IO, page cache) during the
concurrent run. Compare foreground p99 with vs without the concurrent scan.

### Axis 6 — snapshot semantics under churn
Repeat axis 1 against the `snapshots/<name>` path while a background
`nodetool compact` runs on the same node. Confirm correctness (still matches
the axis-5 baseline count) and note latency variance.

## 7. Profile anything surprising

```bash
# Rust side (cqlite-flight) — perf is available on the lab AMI
easy-db-lab ssh <node> -- "sudo perf record -p <cqlite-flight-pid> -g -- sleep 30"
scripts/flamegraph.sh   # this repo's portable perf/dtrace wrapper, if applicable to the captured data

# JVM side (Trino/Cassandra) — async-profiler is in the lab AMI
easy-db-lab ssh <node> -- "asprof -d 30 -f /tmp/trino-profile.html <trino-pid>"
```

## 8. Record results and tear down

```bash
# Pull raw outputs before destroying anything
mkdir -p reports/flight-bench-$(date +%F)
# ... copy timing logs, Grafana screenshots, profile outputs here ...

easy-db-lab down   # same-session teardown — cost discipline
```

Fill in the numbers table (six axes) in this doc's own follow-up section (or
a dated `reports/flight-bench-YYYY-MM-DD/RESULTS.md`) once a real run
completes, and file upstream findings per Reference D — confirmations count
too (e.g. axis 3/4 numbers are good context for pmcfadin/cqlite#1336).

## Open items to confirm before the first real run

- Current NVMe-backed EC2 instance type/pricing (`i4i.xlarge` above is a
  placeholder — check `easy-db-lab show-iam-policies`/current docs).
- Exact `in.mcfad:cqlite-trino` connector catalog property names and its
  Trino version compatibility at benchmark time.
- Whether `easy-db-lab`'s K3s setup needs `hostNetwork: true` for the
  DaemonSet or has its own recommended per-node service-exposure pattern —
  confirm against current easy-db-lab docs rather than assuming.
- Estimated cost for a working session (4 instances × hours) — compute and
  record actual $ once run, to inform the recurring-cadence decision (issue
  #36's final deliverable).
