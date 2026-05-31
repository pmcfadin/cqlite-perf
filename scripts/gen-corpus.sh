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

echo "==> Loading ${ROWS} rows"
# Server-side generation: a single CQL loop would be slow over cqlsh, so we batch
# via a generated COPY-style insert script. For S tier a simple loop suffices.
python3 - "$CONTAINER" "$KEYSPACE" "$SCHEMA" "$ROWS" <<'PY'
import subprocess, sys
container, ks, table, rows = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
BATCH = 1000
payload = "x" * 256
buf = []
def flush(stmts):
    script = "\n".join(stmts)
    subprocess.run(["docker", "exec", "-i", container, "cqlsh", "-k", ks],
                   input=script.encode(), check=True)
for i in range(rows):
    buf.append(
        f"INSERT INTO {table} (id,name,email,age,payload) VALUES "
        f"('k{i:016x}','name-{i}','u{i}@perf.test',{i%120},'{payload}');"
    )
    if len(buf) >= BATCH:
        flush(buf); buf = []
        if (i+1) % 50000 == 0: print(f"  loaded {i+1}/{rows}", flush=True)
if buf: flush(buf)
print(f"  loaded {rows}/{rows}")
PY

echo "==> nodetool flush"
docker exec "$CONTAINER" nodetool flush "$KEYSPACE"

echo "==> Copying SSTables to ${OUT_DIR}"
rm -rf "$OUT_DIR"; mkdir -p "$OUT_DIR"
docker cp "$CONTAINER:/var/lib/cassandra/data/$KEYSPACE/." "$OUT_DIR/"

# --- manifest ---------------------------------------------------------------
echo "==> Computing SHA-256 + writing manifest"
SHA="$(
	find "$OUT_DIR" -name '*-Data.db' -print0 | sort -z |
		xargs -0 cat | shasum -a 256 | awk '{print $1}'
)"
BYTES="$(find "$OUT_DIR" -type f -printf '%s\n' 2>/dev/null | awk '{s+=$1} END{print s+0}')"
[[ -z "$BYTES" || "$BYTES" == "0" ]] && BYTES="$(du -sk "$OUT_DIR" | awk '{print $1*1024}')"
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
