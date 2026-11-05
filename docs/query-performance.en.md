# Query performance baseline

[简体中文](query-performance.md) | English

Measured on 2026-11-05 in a Windows development VM with approximately 8 GiB RAM, using Rust 1.94 Release. The production matcher, complete-result sorting/grouping, pagination and cancellation run without a 200-result truncation. These are measurements, not performance promises.

## Method

- One million and ten million file records span 12 filesystem caches. Names contain Chinese and English, with nested Chinese parent paths, zero-byte and MiB/GiB-sized files, repeated names/sizes and complete timestamps.
- Each partition shares one deep parent directory. This does not cover numerous branching directories, real long IDs, missing metadata, cold SQLite loading, disk indexing, or complete GUI/idle-service resources. Missing fields, folders and hard links have separate behavioral tests.
- Ordinary queries run 20 times; nearest-rank P95 is the 19th sorted sample. Heavy queries run three times at one million and once at ten million, so their ranges are not P95.
- Counts are checked against independent generator formulas. Timings include complete matching/grouping/sorting and materializing the first 100 rows, excluding IPC, rendering and content reads.
- Caches are constructed in-process. This is not cold startup or whole-service memory. VM scheduling and memory reclamation affect results; working-set trimming occurred after ten-million-row construction.

## Results

| Measurement | 1 million | 10 million |
|---|---:|---:|
| Cache construction | 0.75 s | 9.43 s |
| Cache private commit | 249,765,888 B (about 238 MiB) | 2,488,016,896 B (about 2.32 GiB) |
| Literal query P95, one match | 220 ms | 1,997 ms |
| High-hit query and global sort | 1.30–1.48 s | 31.69 s (one run) |
| Same-size candidates | 1.93–2.93 s | 53.17 s (one run) |
| Joint name/size candidates | 2.23–2.56 s | 38.46 s (one run) |
| PCRE2 query, one match | 0.54–0.69 s | 6.86 s (one run) |
| Peak query working set | About 261 MiB | About 2.54 GiB |
| Cancellation return, including a 150 ms signal delay | 172 ms | 290 ms |

The proposed reference machine has 16 GiB RAM; this VM has about 8 GiB, so this is not a same-environment budget test. Nevertheless, local ordinary queries exceed the proposed 150/300 ms budgets, and the ten-million-row cache alone exceeds the 2 GiB service private-commit target. These budgets have not been demonstrated. Complete results also retain row references, and index updates retaining old snapshots increase peaks.

This iteration replaces string metadata with numeric cache fields, reuses folded names, and constructs response paths only during pagination. The initial ASCII million-row sample used about 426 MB and roughly 1.8 s per ordinary query; after optimization, that same ASCII sample used about 250 MB and roughly 0.1 s. Prefer the final mixed-language results above rather than selecting faster intermediate numbers for promotion.

Candidate retrieval, sorting and cache layout still need optimization, followed by reference-hardware measurements of the whole service, cold databases, GUI processes and idle use. Completeness, full-scope duplicate grouping and explicit cancellation remain priorities; results are not truncated to improve timings.

## Reproduce

Run from the repository root without concurrent builds, browser tests or disk scans:

```powershell
cargo build -p stashfinder-store --bin query-baseline --features query-bench --release --locked
.\target\release\query-baseline.exe 1000000
.\target\release\query-baseline.exe 10000000
```

JSON lines include all timing samples, counts, memory and cancellation. The benchmark creates no catalog and writes no source disks. The synthetic constructor is available only with the explicit `query-bench` feature and is absent from ordinary builds.
