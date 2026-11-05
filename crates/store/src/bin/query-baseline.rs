use anyhow::{Result, ensure};
use stashfinder_core::search::*;
use stashfinder_store::search::query::{ResultSet, synthetic_sources};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
use windows::Win32::System::{
    ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
    Threading::GetCurrentProcess,
};
fn memory() -> Result<serde_json::Value> {
    let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
    // SAFETY: the buffer is an initialized PROCESS_MEMORY_COUNTERS_EX of the declared size.
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )?;
    }
    Ok(
        serde_json::json!({"private_bytes":c.PrivateUsage,"working_set_bytes":c.WorkingSetSize,"peak_working_set_bytes":c.PeakWorkingSetSize}),
    )
}
fn main() -> Result<()> {
    let count: usize = std::env::args()
        .nth(1)
        .unwrap_or("1000000".into())
        .parse()?;
    ensure!(
        [1000000, 10000000].contains(&count),
        "Use 1000000 or 10000000 rows"
    );
    let start = Instant::now();
    let sources = synthetic_sources(count);
    let load = start.elapsed().as_secs_f64();
    println!(
        "{}",
        serde_json::json!({"count":count,"construct_seconds":load,"memory":memory()?})
    );
    for text in [
        "rare-00000001",
        "file",
        "dupesize:",
        "dupe:name;size",
        r#"regex:"rare-[0-9]+\.mkv$""#,
    ] {
        let o = QueryOptions::default();
        let q = Query::compile(text, false, &o)?;
        let mut times = Vec::new();
        let mut total = 0;
        for _ in 0..if text == "rare-00000001" {
            20
        } else if count == 10000000 {
            1
        } else {
            3
        } {
            let start = Instant::now();
            let r = ResultSet::run(sources.clone(), &q, &o, &AtomicBool::new(false))?;
            total = r.len();
            let _page = r.page(0, 100)?;
            times.push(start.elapsed().as_secs_f64());
            ensure!(
                total
                    == if text == "rare-00000001" || text.starts_with("regex:") {
                        1
                    } else if text == "dupesize:" {
                        count
                    } else {
                        count - 1
                    },
                "Result count differs from independent generator expectation"
            );
        }
        let p95 = if times.len() == 20 {
            let mut ordered = times.clone();
            ordered.sort_by(f64::total_cmp);
            Some(ordered[18])
        } else {
            None
        };
        println!(
            "{}",
            serde_json::json!({"query":text,"total":total,"seconds":times,"p95_seconds":p95,"memory":memory()?})
        );
    }
    let stop = Arc::new(AtomicBool::new(true));
    let q = Query::compile("", false, &Default::default())?;
    let start = Instant::now();
    ensure!(
        ResultSet::run(sources.clone(), &q, &Default::default(), &stop).is_err(),
        "Cancellation failed"
    );
    println!(
        "{}",
        serde_json::json!({"cancel_ms":start.elapsed().as_secs_f64()*1000.0,"note":"12 synthetic production caches; ordinary 20 runs, heavy 1/3 runs; not cold SQLite loading"})
    );
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        worker_stop.store(true, std::sync::atomic::Ordering::Release);
    });
    let q = Query::compile("file", false, &Default::default())?;
    let start = Instant::now();
    ensure!(
        ResultSet::run(sources.clone(), &q, &Default::default(), &stop).is_err(),
        "Active cancellation failed"
    );
    worker.join().unwrap();
    println!(
        "{}",
        serde_json::json!({"active_cancel_ms":start.elapsed().as_secs_f64()*1000.0,"signal_after_ms":150})
    );
    Ok(())
}
