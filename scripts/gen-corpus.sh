#!/usr/bin/env bash
# gen-corpus.sh — generate an authentic Cassandra 5.0 SSTable corpus for the
# read suite (SPEC §8.1), then emit a manifest + SHA-256 (SPEC §8.3).
#
# Usage:
#   scripts/gen-corpus.sh --tier S --codec lz4 --schema basic
#
# Produces:
#   datasets/<id>/                  SSTable files (gitignored)
#   datasets/manifests/<id>.json    manifest (committed)
#
# Requires Docker. The dataset binaries are never committed — only the manifest.
set -euo pipefail

# --- args -------------------------------------------------------------------
TIER="S"
CODEC="lz4"
SCHEMA="basic"
while [[ $# -gt 0 ]]; do
	case "$1" in
	--tier) TIER="$2"; shift 2 ;;
	--codec) CODEC="$2"; shift 2 ;;
	--schema) SCHEMA="$2"; shift 2 ;;
	*) echo "unknown arg: $1" >&2; exit 2 ;;
	esac
done

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KEYSPACE="perf"
ID="read-${SCHEMA}-${TIER}-${CODEC}"
CONTAINER="cqlite-perf-gen-${ID}"
OUT_DIR="${REPO_ROOT}/datasets/${ID}"
MANIFEST="${REPO_ROOT}/datasets/manifests/${ID}.json"
SCHEMA_FILE="${REPO_ROOT}/schemas/${SCHEMA}.cql"
IMAGE="cassandra:5.0"
GEN_VERSION="5.0.0"

# Rows per tier (SPEC §8.4). S kept modest so a smoke corpus builds in minutes.
case "$TIER" in
S) ROWS=100000 ;;
M) ROWS=25000000 ;;
L) ROWS=100000000 ;;
*) echo "unknown tier: $TIER" >&2; exit 2 ;;
esac

# Map our codec name to the Cassandra compressor class (SPEC §8.1).
case "$CODEC" in
none) COMPRESSION="compression = {}" ;;
lz4) COMPRESSION="compression = {'class':'LZ4Compressor'}" ;;
snappy) COMPRESSION="compression = {'class':'SnappyCompressor'}" ;;
deflate) COMPRESSION="compression = {'class':'DeflateCompressor'}" ;;
zstd) COMPRESSION="compression = {'class':'ZstdCompressor'}" ;;
*) echo "unknown codec: $CODEC" >&2; exit 2 ;;
esac

[[ -f "$SCHEMA_FILE" ]] || { echo "schema not found: $SCHEMA_FILE" >&2; exit 1; }

# --- cassandra lifecycle ----------------------------------------------------
cql() { docker exec "$CONTAINER" cqlsh -e "$1"; }

wait_for_cassandra() {
	echo "Waiting for Cassandra to accept CQL..."
	for _ in $(seq 1 60); do
		if docker exec "$CONTAINER" cqlsh -e "SELECT now() FROM system.local" >/dev/null 2>&1; then
			echo "Cassandra is up."
			return 0
		fi
		sleep 5
	done
	echo "Cassandra did not become ready in time" >&2
	return 1
}

cleanup() { docker rm -f "$CONTAINER" >/dev/null 2>&1 || true; }
trap cleanup EXIT

echo "==> Starting Cassandra ($IMAGE) for ${ID} (${ROWS} rows)"
cleanup
docker run -d --name "$CONTAINER" "$IMAGE" >/dev/null
wait_for_cassandra

echo "==> Creating keyspace + table (codec=${CODEC})"
cql "CREATE KEYSPACE IF NOT EXISTS ${KEYSPACE} WITH replication =
     {'class':'SimpleStrategy','replication_factor':1};"
# The committed schema is unqualified; create it under our keyspace, applying the
# codec to the table via ALTER after creation for codec independence.
docker cp "$SCHEMA_FILE" "$CONTAINER:/schema.cql"
docker exec "$CONTAINER" cqlsh -k "$KEYSPACE" -f /schema.cql
cql "ALTER TABLE ${KEYSPACE}.${SCHEMA} WITH ${COMPRESSION};"

# Column order per schema for the COPY statement (must match the CSV writer
# below and the table's declared columns).
case "$SCHEMA" in
basic)            COLS="id,name,email,age,payload" ;;
collections)      COLS="id,tags,scores,props,payload" ;;
wide_rows)        COLS="pk,ck,val" ;;
*) echo "no COPY column map for schema: $SCHEMA" >&2; exit 2 ;;
esac

echo "==> Generating CSV (${ROWS} rows, schema=${SCHEMA})"
# Bulk load via cqlsh COPY FROM instead of a per-batch cqlsh spawn (issue #12).
# We generate a pipe-delimited CSV host-side, then stream it through one COPY —
# orders of magnitude faster than the old per-1000-row `docker exec cqlsh` loop,
# and the only path that makes the M/L tiers feasible. The delimiter is `|` so
# collection literals (which contain commas) need no escaping.
CSV_HOST="$(mktemp -t cqlite-perf-${SCHEMA}-XXXXXX.csv)"
python3 - "$SCHEMA" "$ROWS" "$CSV_HOST" <<'PY'
import sys
schema, rows, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]

def basic():
    payload = "x" * 256
    for i in range(rows):
        yield f"k{i:016x}|name-{i}|u{i}@perf.test|{i % 120}|{payload}\n"

def collections():
    # set<text>, list<int>, map<text,text> + a small payload. Exercises
    # collection deserialization cost (read.type_heavy, SPEC §6).
    payload = "x" * 64
    for i in range(rows):
        tags   = "{" + ",".join(f"t{i}_{j}" for j in range(3)) + "}"
        scores = "[" + ",".join(str((i + j) % 100) for j in range(4)) + "]"
        props  = "{" + f"a:v{i},b:w{i}" + "}"
        yield f"k{i:016x}|{tags}|{scores}|{props}|{payload}\n"

def wide_rows():
    # Few partitions, many clustering rows each → genuinely wide partitions
    # (read.wide_partition + read.clustering_slice, SPEC §6).
    parts = 100
    per = max(1, rows // parts)
    val = "x" * 128
    n = 0
    for pk in range(parts):
        for ck in range(per):
            if n >= rows:
                return
            yield f"p{pk:04d}|{ck}|{val}\n"
            n += 1

gens = {"basic": basic, "collections": collections, "wide_rows": wide_rows}
g = gens.get(schema)
if g is None:
    sys.stderr.write(f"no CSV generator for schema {schema}\n")
    sys.exit(2)
with open(out, "w") as f:
    for line in g():
        f.write(line)
PY

echo "==> Loading via cqlsh COPY FROM (cols: ${COLS})"
docker cp "$CSV_HOST" "$CONTAINER:/corpus.csv"
rm -f "$CSV_HOST"
# DATETIMEFORMAT/NULL left at defaults; HEADER=false since we emit raw rows.
cql "COPY ${KEYSPACE}.${SCHEMA} (${COLS}) FROM '/corpus.csv'
     WITH DELIMITER='|' AND HEADER=false;"

# Verify the row count actually landed — a silent COPY shortfall would poison
# every downstream rows/sec number (the M1 0-rows class of bug).
LOADED="$(cql "SELECT COUNT(*) FROM ${KEYSPACE}.${SCHEMA};" | sed -n '4p' | tr -d ' ')"
echo "  loaded ${LOADED}/${ROWS} rows"
if [[ "$LOADED" != "$ROWS" ]]; then
	echo "row count mismatch: loaded ${LOADED} != expected ${ROWS}" >&2
	exit 1
fi

echo "==> nodetool flush"
docker exec "$CONTAINER" nodetool flush "$KEYSPACE"

echo "==> Copying SSTables to ${OUT_DIR}"
# Preserve the keyspace directory level so SSTable discovery infers the correct
# keyspace (datasets/<id>/<keyspace>/<table>-<uuid>/...). Copying the keyspace
# dir's *contents* instead would make discovery mis-infer the keyspace from the
# dataset dir name, and queries silently return zero rows.
rm -rf "$OUT_DIR"; mkdir -p "$OUT_DIR"
docker cp "$CONTAINER:/var/lib/cassandra/data/$KEYSPACE" "$OUT_DIR/"

# --- manifest ---------------------------------------------------------------
echo "==> Computing SHA-256 + writing manifest"
SHA="$(
	find "$OUT_DIR" -name '*-Data.db' -print0 | sort -z |
		xargs -0 cat | shasum -a 256 | awk '{print $1}'
)"
# Portable byte count (macOS `find` lacks -printf): du -sk gives KiB blocks.
BYTES="$(du -sk "$OUT_DIR" | awk '{print $1*1024}')"
NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

mkdir -p "$(dirname "$MANIFEST")"
cat >"$MANIFEST" <<JSON
{
  "id": "${ID}",
  "tier": "${TIER}",
  "schema": "${SCHEMA}",
  "codec": "${CODEC}",
  "rows": ${ROWS},
  "bytes": ${BYTES},
  "generator": "cassandra-5.0",
  "generator_version": "${GEN_VERSION}",
  "cqlite_schema_ref": "schemas/${SCHEMA}.cql",
  "sha256": "${SHA}",
  "created_utc": "${NOW}",
  "path_hint": "datasets/${ID}/"
}
JSON

echo "==> Done: ${ID}"
echo "    data:     ${OUT_DIR}"
echo "    manifest: ${MANIFEST}"
echo "    sha256:   ${SHA}"
