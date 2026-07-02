//! Repro: `WriteEngine::maintenance_step` panics when called from a tokio
//! worker thread (cqlite v0.10.0). Filed upstream as cqlite #587.
//!
//! `maintenance_step` is a *sync* method, but once a merge has input SSTables to
//! read it bridges to async internally via `block_on_async`
//! (write_engine/merge.rs), which does `Handle::current().block_on(..)`. Called
//! from inside any tokio runtime worker that panics with:
//!
//!   "Cannot start a runtime from within a runtime ... (like `block_on`) ..."
//!
//! So compaction is unreachable from an async context (the normal way to drive
//! the engine, since `flush()`/`close()` are async). Workaround: hop onto a
//! non-runtime thread (`spawn_blocking`) so the engine takes its own-runtime
//! branch — but a sync API shouldn't require callers to know that.
//!
//! Run: `cargo run --example probe_compaction`
//! Expected (bug): panics at maintenance_step. With the spawn_blocking hop
//! (second half) it completes and merges 4 SSTables → 1.

use std::time::Duration;

use cqlite_core::schema::{Column, KeyColumn, TableSchema};
use cqlite_core::storage::write_engine::merge_policy::STCSPolicy;
use cqlite_core::storage::write_engine::mutation::{CellOperation, Mutation, PartitionKey, TableId};
use cqlite_core::storage::write_engine::{Durability, WriteEngine, WriteEngineConfig};
use cqlite_core::types::Value;

const KS: &str = "perf";
const TBL: &str = "ingest";
const THRESHOLD: usize = 1024 * 1024; // 1 MB SSTables, so the repro is quick

fn schema() -> TableSchema {
    let col = |n: &str| Column {
        name: n.to_string(),
        data_type: "text".to_string(),
        nullable: false,
        default: None,
        is_static: false,
    };
    TableSchema {
        keyspace: KS.to_string(),
        table: TBL.to_string(),
        partition_keys: vec![KeyColumn { name: "id".to_string(), data_type: "text".to_string(), position: 0 }],
        clustering_keys: vec![],
        columns: vec![col("id"), col("payload")],
        comments: Default::default(),
        dropped_columns: Default::default(),
    }
}

fn mutation(n: u64) -> Mutation {
    Mutation::new(
        TableId::new(KS, TBL),
        PartitionKey::new(vec![("id".to_string(), Value::Text(format!("k{n:016x}")))]),
        None,
        vec![CellOperation::Write { column: "payload".to_string(), value: Value::Text("x".repeat(256)) }],
        (n as i64) + 1,
        None,
    )
}

async fn build_four_sstables(dir: &std::path::Path) -> anyhow::Result<WriteEngine> {
    let data = dir.join("data");
    let wal = dir.join("wal");
    std::fs::create_dir_all(&data)?;
    std::fs::create_dir_all(&wal)?;
    let cfg = WriteEngineConfig::new(data, wal, schema())
        .with_flush_threshold(THRESHOLD)
        .with_hard_limit(64 * 1024 * 1024)
        .with_durability(Durability::Disabled);
    let mut e = WriteEngine::new(cfg).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut n = 0u64;
    for _ in 0..4 {
        while e.memtable_size() < THRESHOLD {
            e.write(mutation(n)).map_err(|e| anyhow::anyhow!("{e}"))?;
            n += 1;
        }
        e.flush().await.map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    let policy = STCSPolicy::new(2, 32, 0.5, 1.5, STCSPolicy::DEFAULT_MIN_SSTABLE_SIZE)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    e.set_merge_policy(Box::new(policy)).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(e)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // --- 1. Direct call from a tokio worker thread → PANICS ---
    let dir1 = tempfile::tempdir()?;
    let mut e = build_four_sstables(dir1.path()).await?;
    println!("built 4 SSTables; calling maintenance_step directly on the runtime thread...");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        e.maintenance_step(Duration::from_secs(5))
    }));
    match result {
        Ok(Ok(report)) => println!("  unexpected success: pending={}", report.pending_compaction),
        Ok(Err(err)) => println!("  returned Err: {err}"),
        Err(_) => println!("  >>> PANICKED (the bug): block_on from within the tokio runtime"),
    }

    // --- 2. Same call hopped onto a non-runtime thread → WORKS ---
    let dir2 = tempfile::tempdir()?;
    let e2 = build_four_sstables(dir2.path()).await?;
    let data2 = dir2.path().join("data");
    println!("hopping maintenance_step onto spawn_blocking...");
    let merged = tokio::task::spawn_blocking(move || {
        let mut e2 = e2;
        loop {
            let r = e2.maintenance_step(Duration::from_secs(5)).expect("maintenance_step");
            if !r.pending_compaction { break; }
        }
    })
    .await;
    merged.map_err(|e| anyhow::anyhow!("join: {e}"))?;
    let n_data = std::fs::read_dir(data2.join(KS).join(TBL))
        .map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().ends_with("-Data.db")).count())
        .unwrap_or(0);
    println!("  workaround OK: compaction converged, Data.db files now = {n_data} (was 4)");
    Ok(())
}
