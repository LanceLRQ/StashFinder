use anyhow::{Result, ensure};
use serde_json::json;
use stashfinder_store::{Cache, create_synthetic, reopen, search_sql};
use std::{hint::black_box, path::PathBuf, time::Instant};
use windows::Win32::System::{
    ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
    Threading::GetCurrentProcess,
};

fn memory() -> Result<serde_json::Value> {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    // SAFETY: EX begins with PROCESS_MEMORY_COUNTERS and cb covers the allocated value.
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )?;
    }
    Ok(
        json!({"private_bytes": counters.PrivateUsage, "working_set_bytes": counters.WorkingSetSize, "peak_working_set_bytes": counters.PeakWorkingSetSize}),
    )
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ensure!(args.len() == 2, "usage: baseline <new.sqlite> <row-count>");
    let path = PathBuf::from(&args[0]);
    let count: usize = args[1].parse()?;
    let start = Instant::now();
    let db = create_synthetic(&path, count)?;
    let import_ms = start.elapsed().as_secs_f64() * 1000.0;
    let after_import = memory()?;
    let sqlite_version: String = db.query_row("SELECT sqlite_version()", [], |r| r.get(0))?;
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(integrity == "ok", "integrity check: {integrity}");
    drop(db);
    let start = Instant::now();
    let db = reopen(&path)?;
    let reopen_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let cache = Cache::load(&db)?;
    let cache_load_ms = start.elapsed().as_secs_f64() * 1000.0;
    let after_cache = memory()?;
    let mut queries = Vec::new();
    for query in [
        "aaa bbb",
        "视频",
        "not-present-xyz",
        "group-0000\\rare",
        "a",
    ] {
        let start = Instant::now();
        let expected = search_sql(&db, query, 200)?;
        let first_sql_ms = start.elapsed().as_secs_f64() * 1000.0;
        ensure!(
            cache.search(query, 200) == expected,
            "SQL/cache mismatch: {query}"
        );
        let mut sql_ms = Vec::new();
        let mut cache_ms = Vec::new();
        for _ in 0..20 {
            let start = Instant::now();
            black_box(search_sql(&db, black_box(query), 200)?);
            sql_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            black_box(cache.search(black_box(query), 200));
            cache_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        sql_ms.sort_by(f64::total_cmp);
        cache_ms.sort_by(f64::total_cmp);
        eprintln!(
            "rows={count} query={query:?} sql_p95={:.3}ms cache_p95={:.3}ms",
            sql_ms[18], cache_ms[18]
        );
        queries.push(json!({"query": query, "returned": expected.len(), "first_sql_ms": first_sql_ms, "sql_p95_ms": sql_ms[18], "cache_p95_ms": cache_ms[18], "sql_samples_ms": sql_ms, "cache_samples_ms": cache_ms}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "rows": count, "volumes": 12, "directories": 1200, "limit": 200, "warm_repetitions": 20,
            "sqlite_version": sqlite_version, "integrity_check": integrity,
            "import_ms": import_ms, "reopen_ms": reopen_ms, "cache_load_ms": cache_load_ms,
            "database_bytes": std::fs::metadata(&path)?.len(), "after_import": after_import,
            "after_cache": after_cache, "after_queries": memory()?, "queries": queries,
            "limitations": "Synthetic data; OS file cache is warm after import. No physical-disk scan, production schema, global sorting, dupe, regex, cancellation, service-idle or GUI budget measured."
        }))?
    );
    Ok(())
}
