//! Read an explicitly selected NTFS volume into a new, isolated catalog.
use anyhow::{Context, Result, ensure};
use stashfinder_core::search::Query;
use stashfinder_service::index::{BuildStrategy, IndexMode, Source, build_with_strategy};
use stashfinder_store::{search::SearchIndex, volume::Volume};
use stashfinder_windows::ntfs::{
    DirectoryEntry, Journal, NativeObject, UsnPage, UsnRecord, VolumeReader, mft::FastScanStats,
};
use std::{
    cell::Cell,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

struct MeasuredSource {
    reader: VolumeReader,
    native: Cell<Duration>,
    consume: Cell<Duration>,
    directories: Cell<u64>,
    entries: Cell<u64>,
    inventory: Cell<u64>,
    started: Instant,
    reported: Cell<Instant>,
}

impl Source for MeasuredSource {
    fn verify(&self) -> Result<()> {
        self.reader.verify()
    }
    fn root_id(&self) -> u64 {
        self.reader.root_id
    }
    fn journal(&self) -> Result<Option<Journal>> {
        self.reader.journal()
    }
    fn create_journal(&self) -> Result<Journal> {
        anyhow::bail!("Benchmark must not create a journal")
    }
    fn full_inventory(
        &self,
        cancel: &AtomicBool,
        consume: &mut dyn FnMut(&[NativeObject]) -> Result<()>,
    ) -> Result<Option<FastScanStats>> {
        self.reader.full_inventory(cancel, consume)
    }
    fn inventory(
        &self,
        cancel: &AtomicBool,
        consume: &mut dyn FnMut(&[UsnRecord]) -> Result<()>,
    ) -> Result<()> {
        let start = Instant::now();
        let mut consumer = Duration::ZERO;
        let result = self.reader.enumerate_mft(cancel, |rows| {
            self.inventory.set(self.inventory.get() + rows.len() as u64);
            let start = Instant::now();
            let result = consume(rows);
            consumer += start.elapsed();
            result
        });
        println!(
            "{}",
            serde_json::json!({"event":"inventory", "elapsed_ms":start.elapsed().as_millis(), "consumer_ms":consumer.as_millis(), "records":self.inventory.get()})
        );
        result
    }
    fn directory(
        &self,
        id: u64,
        cancel: &AtomicBool,
        consume: &mut dyn FnMut(&[DirectoryEntry]) -> Result<()>,
    ) -> Result<()> {
        let start = Instant::now();
        let mut consumer = Duration::ZERO;
        let result = self.reader.directory(id, cancel, |rows| {
            self.entries.set(self.entries.get() + rows.len() as u64);
            let start = Instant::now();
            let result = consume(rows);
            consumer += start.elapsed();
            result
        });
        self.native
            .set(self.native.get() + start.elapsed() - consumer);
        self.consume.set(self.consume.get() + consumer);
        self.directories.set(self.directories.get() + 1);
        if self.reported.get().elapsed() >= Duration::from_secs(5) {
            println!(
                "{}",
                serde_json::json!({"event":"scan", "elapsed_ms":self.started.elapsed().as_millis(), "directories":self.directories.get(), "entries":self.entries.get(), "native_ms":self.native.get().as_millis(), "consumer_ms":self.consume.get().as_millis()})
            );
            self.reported.set(Instant::now());
        }
        result
    }
    fn object(&self, id: u64) -> Result<Option<NativeObject>> {
        self.reader.lookup_object(id)
    }
    fn changes(&self, journal: u64, start: i64) -> Result<UsnPage> {
        self.reader.changes(journal, start)
    }
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    ensure!(
        (3..=4).contains(&args.len()),
        "Usage: index-benchmark <volume GUID path> <new database path> <timeout seconds> [auto|legacy|sequential]"
    );
    let strategy = match args.get(3).map(String::as_str).unwrap_or("auto") {
        "auto" => BuildStrategy::Auto,
        "legacy" => BuildStrategy::Legacy,
        "sequential" => BuildStrategy::Sequential,
        other => anyhow::bail!("Unknown index strategy: {other}"),
    };
    let path = PathBuf::from(&args[1]);
    ensure!(
        path.is_absolute() && !path.exists(),
        "Use a new absolute database path"
    );
    let timeout = args[2].parse::<u64>()?;
    ensure!(timeout > 0, "Timeout must be positive");
    stashfinder_windows::portable::enable_backup_read()?;
    let reader = VolumeReader::open(&args[0])?;
    reader
        .journal()?
        .context("Benchmark requires an existing USN journal")?;
    let cancel = Arc::new(AtomicBool::new(false));
    let timer_cancel = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(timeout));
        timer_cancel.store(true, Ordering::Relaxed);
    });
    let started = Instant::now();
    let source = MeasuredSource {
        reader,
        native: Cell::new(Duration::ZERO),
        consume: Cell::new(Duration::ZERO),
        directories: Cell::new(0),
        entries: Cell::new(0),
        inventory: Cell::new(0),
        started,
        reported: Cell::new(started),
    };
    let mut db = Volume::create(&path, "{}")?;
    let result = build_with_strategy(
        &source,
        &mut db,
        IndexMode::Journal,
        strategy,
        &cancel,
        &mut |phase| {
            println!(
                "{}",
                serde_json::json!({"event":"phase", "phase":phase, "elapsed_ms":started.elapsed().as_millis()})
            );
        },
    );
    println!(
        "{}",
        serde_json::json!({"event":"build", "elapsed_ms":started.elapsed().as_millis(), "directories":source.directories.get(), "entries":source.entries.get(), "native_ms":source.native.get().as_millis(), "consumer_ms":source.consume.get().as_millis(), "error":result.as_ref().err().map(|e|format!("{e:#}"))})
    );
    let report = result?;
    let scan = report.scan.map(|scan| {
        serde_json::json!({
            "read_calls":scan.read_calls, "read_bytes":scan.read_bytes,
            "read_ms":scan.read_time.as_millis(), "parse_ms":scan.parse_time.as_millis(),
            "consumer_ms":scan.consumer_time.as_millis(), "auxiliary_reads":scan.auxiliary_reads,
            "base_records":scan.base_records, "extension_records":scan.extension_records,
            "objects":scan.objects, "links":scan.links, "peak_cache_bytes":scan.peak_cache_bytes,
        })
    });
    println!(
        "{}",
        serde_json::json!({
            "event":"strategy", "requested":format!("{strategy:?}"), "actual":format!("{:?}",report.strategy),
            "fallback":report.fallback, "scan":scan, "stage_ms":report.stage_time.as_millis(),
            "import_ms":report.import_time.as_millis(), "replay_ms":report.replay_time.as_millis(),
            "validate_ms":report.validate_time.as_millis(),
        })
    );
    let loaded = Instant::now();
    let index = SearchIndex::load(&db)?.context("No published index")?;
    println!(
        "{}",
        serde_json::json!({"event":"load", "elapsed_ms":loaded.elapsed().as_millis(), "total_ms":started.elapsed().as_millis(), "records":db.index_status()?.records})
    );
    for query in ["", "ext:jpg", "StashFinder"] {
        let search = Instant::now();
        let result = index.search(&Query::parse(query, false)?, &[], 200, &cancel)?;
        println!(
            "{}",
            serde_json::json!({"event":"search", "query":query, "elapsed_us":search.elapsed().as_micros(), "rows":result.rows.len(), "more":result.more})
        );
    }
    Ok(())
}
