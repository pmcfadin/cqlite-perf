# cqlite-perf — project guidance for Claude

External macro-benchmark harness for **CQLite**. It depends on `cqlite-core` by
git tag (currently **v0.9.2**). The engine is **not in this repo** — it lives at
<https://github.com/pmcfadin/cqlite> (`pmcfadin/cqlite`, default branch `main`).

## Filing engine bugs upstream (important)

When benchmarking surfaces a bug in the **engine** — wrong or zero rows, ignored
clauses, crashes, perf cliffs — it must be fixed in `pmcfadin/cqlite`, not here.

> **Never push code to `pmcfadin/cqlite`.** No commits, branches, or PRs to the
> engine repo. Engagement is limited to **issues and comments** — report the bug
> with a repro and let the engine team write the fix.

File it there with `gh`:

```
gh issue list   -R pmcfadin/cqlite --search "<keywords>"   # search first — avoid dups
gh issue create -R pmcfadin/cqlite --title "..." --body "..."
gh issue comment <n> -R pmcfadin/cqlite --body "..."        # add evidence to an existing one
```

Rules of thumb:
- **Search before filing**; if a related issue exists, comment with added evidence
  instead of duplicating.
- **Always include a minimal repro.** An `examples/probe_*.rs` in this repo that a
  maintainer can `cargo run --example` is ideal.
- **State plainly whether it's an engine bug or a harness/data issue**, and give
  the evidence that rules out the harness (e.g. "the stored key bytes decode to
  exactly the generated key; the same code counts N rows on a full scan").
- **Cross-link**: note the upstream issue number on the corresponding cqlite-perf
  tracking issue so the blocker stays visible on this side.

## Environment

- Rust is via rustup but **not on PATH**: prefix shells with
  `export PATH="$HOME/.cargo/bin:$PATH"`.
- `cqlite-core` is a git-tag dep (no local path); cold builds are ~1 min.
- Corpus generation needs Docker (`cassandra:5.0`). Datasets are gitignored —
  only manifests under `datasets/manifests/` are committed. `scripts/gen-corpus.sh`
  bulk-loads via `cqlsh COPY FROM`.
- A workload is **not done until it has run against a real corpus and emitted
  rows** — compiling is not enough.

## Known upstream blockers (v0.9.2)

- **cqlite #548** — `WHERE pk = ?` returns 0 rows: the partition-key column is not
  materialized into `SELECT` result rows. Blocks `read.point_lookup` and
  `read.clustering_slice` (tracked here as #13). Repro: `cargo run --example probe_cols`.
- **cqlite #581** — `LIMIT` ignored on the streaming path. Repro: `cargo run --example probe_point`.

## Commit trailer

```
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```
