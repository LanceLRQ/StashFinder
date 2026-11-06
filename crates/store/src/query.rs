use super::SearchIndex;
#[cfg(feature = "query-bench")]
pub fn synthetic_sources(count: usize) -> Vec<Source> {
    (0..12)
        .map(|part| {
            let start = count * part / 12;
            synthetic_range(start, count * (part + 1) / 12 - start)
        })
        .collect()
}
#[cfg(feature = "query-bench")]
fn synthetic_range(start: usize, count: usize) -> Source {
    use super::{CachedEntry, CachedRow};
    let entries = (start..start + count)
        .map(|i| {
            let name = if i == 1 {
                "rare-00000001.mkv".into()
            } else {
                format!(
                    "file-{}{:06}.mkv",
                    if i % 2 == 0 { "" } else { "照片-" },
                    i % 100000
                )
            };
            CachedEntry {
                parent: 0,
                row: CachedRow {
                    entry: (i as u64).to_le_bytes().to_vec().into_boxed_slice(),
                    object: (i as u64).to_le_bytes().to_vec().into_boxed_slice(),
                    folded: name.to_lowercase().into_boxed_str(),
                    name: name.into_boxed_str(),
                    kind: "file",
                    attributes: 0,
                    size: Some((i % 10000) as u64 * 1048576),
                    created: Some(133000000000000000),
                    modified: Some(133000000000000000),
                },
            }
        })
        .collect();
    Source {
        id: format!("synthetic-{start}"),
        name: "Synthetic".into(),
        filesystem: format!("synthetic-{start}"),
        connection: "offline".into(),
        mounts: vec![],
        needs_validation: false,
        index: Arc::new(SearchIndex {
            generation: 1,
            revision: 0,
            partial: false,
            indexed_at: Some(1),
            needs_validation: false,
            directories: vec!["项目\\aaa\\level-1\\level-2\\深目录".into()],
            folded_directories: vec!["项目\\aaa\\level-1\\level-2\\深目录".into()],
            directory_entries: vec![vec![0]],
            entries,
            directory_ids: std::collections::HashMap::from([(vec![0], 0)]),
            links: std::collections::HashSet::new(),
        }),
    }
}
use anyhow::{Context, Result, ensure};
use stashfinder_core::{index::SearchHit, search::*};
use std::{
    cmp::Ordering as Cmp,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub filesystem: String,
    pub connection: String,
    pub mounts: Vec<String>,
    pub index: Arc<SearchIndex>,
    pub needs_validation: bool,
}
#[derive(Clone, Copy, Default)]
struct Row {
    source: u32,
    entry: u32,
    group: u32,
}
pub struct ResultSet {
    pub sources: Vec<Source>,
    rows: Vec<Row>,
    pub groups: usize,
    pub missing: usize,
}
impl ResultSet {
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn run(
        sources: Vec<Source>,
        query: &Query,
        options: &QueryOptions,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        let mut result = Self {
            sources,
            rows: Vec::new(),
            groups: 0,
            missing: 0,
        };
        for (s, source) in result.sources.iter().enumerate() {
            let scope = options
                .scope
                .as_ref()
                .filter(|scope| scope.partition_id == source.id);
            if options.scope.is_some() && scope.is_none() {
                continue;
            }
            let prefix = scope
                .map(|scope| {
                    source
                        .index
                        .directory_ids
                        .get(&scope.entry)
                        .map(|i| source.index.directories[*i].as_str())
                        .context("搜索目录已不存在")
                })
                .transpose()?;
            for (i, entry) in source.index.entries.iter().enumerate() {
                ensure!(!cancel.load(Ordering::Relaxed), "搜索已取消");
                if let Some(scope) = scope {
                    if !scope.recursive
                        && source.index.directory_entries[entry.parent] != scope.entry
                    {
                        continue;
                    }
                    let parent = &source.index.directories[entry.parent];
                    let prefix = prefix.unwrap();
                    if scope.recursive
                        && !prefix.is_empty()
                        && parent != prefix
                        && !parent
                            .strip_prefix(prefix)
                            .is_some_and(|p| p.starts_with('\\'))
                    {
                        continue;
                    }
                }
                let row = &entry.row;
                if query.matches_record(
                    &row.record(
                        &source.index.directories[entry.parent],
                        &source.index.folded_directories[entry.parent],
                    ),
                    &source.mounts,
                )? {
                    let unknown = query.duplicate().iter().any(|f| match f {
                        DuplicateField::Name => false,
                        DuplicateField::Size => row.kind == "folder" || row.size.is_none(),
                        DuplicateField::Modified => row.modified.is_none(),
                    });
                    if unknown {
                        result.missing += 1;
                        continue;
                    }
                    result.rows.push(Row {
                        source: s.try_into()?,
                        entry: i.try_into()?,
                        group: 0,
                    });
                }
            }
        }
        if !query.duplicate().is_empty() {
            let sources = &result.sources;
            let compare = |a: Row, b: Row| {
                compare_dupe(sources, a, b, query.duplicate(), options.case_sensitive)
            };
            sort(&mut result.rows, compare, cancel)?;
            let mut start = 0;
            let mut kept = Vec::new();
            while start < result.rows.len() {
                ensure!(!cancel.load(Ordering::Relaxed), "搜索已取消");
                let first = result.rows[start];
                let mut end = start + 1;
                while end < result.rows.len() && compare(first, result.rows[end]) == Cmp::Equal {
                    ensure!(!cancel.load(Ordering::Relaxed), "搜索已取消");
                    end += 1;
                }
                let first_source = &sources[first.source as usize];
                let first_object = &first_source.index.entries[first.entry as usize].row.object;
                let distinct = result.rows[start + 1..end].iter().any(|r| {
                    sources[r.source as usize].filesystem != first_source.filesystem
                        || &sources[r.source as usize].index.entries[r.entry as usize]
                            .row
                            .object
                            != first_object
                });
                if distinct {
                    result.groups += 1;
                    for mut r in result.rows[start..end].iter().copied() {
                        ensure!(!cancel.load(Ordering::Relaxed), "搜索已取消");
                        r.group = result.groups.try_into()?;
                        kept.push(r);
                    }
                }
                start = end;
            }
            result.rows = kept;
        }
        let sources = &result.sources;
        sort(
            &mut result.rows,
            |a, b| {
                let sa = &sources[a.source as usize];
                let sb = &sources[b.source as usize];
                let ra = &sa.index.entries[a.entry as usize].row;
                let rb = &sb.index.entries[b.entry as usize].row;
                let cmp = match options.sort {
                    SortField::Name => ra
                        .name_key(options.case_sensitive)
                        .cmp(rb.name_key(options.case_sensitive)),
                    SortField::Path => text_cmp(
                        &sa.index.directories[sa.index.entries[a.entry as usize].parent],
                        &sb.index.directories[sb.index.entries[b.entry as usize].parent],
                        options.case_sensitive,
                    )
                    .then_with(|| {
                        ra.name_key(options.case_sensitive)
                            .cmp(rb.name_key(options.case_sensitive))
                    }),
                    SortField::Partition => text_cmp(&sa.name, &sb.name, options.case_sensitive),
                    SortField::Group => a.group.cmp(&b.group),
                    SortField::Size => number_cmp(&ra.size, &rb.size, options.descending),
                    SortField::Created => number_cmp(&ra.created, &rb.created, options.descending),
                    SortField::Modified => {
                        number_cmp(&ra.modified, &rb.modified, options.descending)
                    }
                };
                let cmp = if options.descending
                    && !matches!(
                        options.sort,
                        SortField::Size | SortField::Created | SortField::Modified
                    ) {
                    cmp.reverse()
                } else {
                    cmp
                };
                cmp.then_with(|| sa.id.cmp(&sb.id))
                    .then_with(|| ra.entry.cmp(&rb.entry))
            },
            cancel,
        )?;
        Ok(result)
    }
    pub fn page(&self, offset: usize, limit: usize) -> Result<Vec<SearchHit>> {
        ensure!(
            offset <= self.rows.len() && limit > 0 && limit <= 100,
            "搜索分页位置或数量无效"
        );
        Ok(self
            .rows
            .iter()
            .skip(offset)
            .take(limit)
            .map(|r| {
                let s = &self.sources[r.source as usize];
                let entry = &s.index.entries[r.entry as usize];
                let file = entry.row.materialize(
                    &s.index.directory_entries[entry.parent],
                    &s.index.directories[entry.parent],
                );
                SearchHit {
                    duplicate_group: (r.group > 0).then_some(r.group),
                    hard_link: s.index.links.contains(&file.object),
                    filesystem_id: s.filesystem.clone(),
                    partition_id: s.id.clone(),
                    partition_name: s.name.clone(),
                    connection: s.connection.clone(),
                    mounts: s.mounts.clone(),
                    generation: s.index.generation,
                    revision: s.index.revision,
                    partial: s.index.partial,
                    indexed_at: s.index.indexed_at,
                    needs_validation: s.index.needs_validation || s.needs_validation,
                    file,
                }
            })
            .collect())
    }
    pub fn reference(&self, offset: usize) -> Result<stashfinder_core::file_operations::EntryRef> {
        let row = self.rows.get(offset).context("所选搜索位置无效")?;
        let source = &self.sources[row.source as usize];
        Ok(stashfinder_core::file_operations::EntryRef {
            partition_id: source.id.clone(),
            filesystem_id: source.filesystem.clone(),
            generation: source.index.generation.to_string(),
            revision: source.index.revision.to_string(),
            entry: source.index.entries[row.entry as usize].row.entry.to_vec(),
        })
    }
}
fn text_cmp(a: &str, b: &str, case: bool) -> Cmp {
    if case {
        a.cmp(b)
    } else {
        a.to_lowercase().cmp(&b.to_lowercase())
    }
}
fn number_cmp(a: &Option<u64>, b: &Option<u64>, descending: bool) -> Cmp {
    match (a, b) {
        (None, None) => Cmp::Equal,
        (None, _) => Cmp::Greater,
        (_, None) => Cmp::Less,
        (Some(a), Some(b)) => {
            let c = a.cmp(b);
            if descending { c.reverse() } else { c }
        }
    }
}
fn compare_dupe(s: &[Source], a: Row, b: Row, fields: &[DuplicateField], case: bool) -> Cmp {
    let a = &s[a.source as usize].index.entries[a.entry as usize].row;
    let b = &s[b.source as usize].index.entries[b.entry as usize].row;
    let mut cmp = a.kind.cmp(b.kind);
    for field in fields {
        if cmp != Cmp::Equal {
            break;
        }
        cmp = match field {
            DuplicateField::Name => a.name_key(case).cmp(b.name_key(case)),
            DuplicateField::Size => number_cmp(&a.size, &b.size, false),
            DuplicateField::Modified => number_cmp(&a.modified, &b.modified, false),
        };
    }
    cmp
}
// Small sorted runs and interruptible merges bound cancellation latency during sorting.
fn sort(rows: &mut Vec<Row>, compare: impl Fn(Row, Row) -> Cmp, cancel: &AtomicBool) -> Result<()> {
    for chunk in rows.chunks_mut(2048) {
        ensure!(!cancel.load(Ordering::Relaxed), "搜索已取消");
        chunk.sort_unstable_by(|a, b| compare(*a, *b));
    }
    let mut width = 2048;
    let mut buffer = vec![Row::default(); rows.len()];
    while width < rows.len() {
        for start in (0..rows.len()).step_by(width * 2) {
            let middle = (start + width).min(rows.len());
            let end = (middle + width).min(rows.len());
            let (mut a, mut b) = (start, middle);
            for output in buffer.iter_mut().take(end).skip(start) {
                ensure!(!cancel.load(Ordering::Relaxed), "搜索已取消");
                if b == end || (a < middle && compare(rows[a], rows[b]) != Cmp::Greater) {
                    *output = rows[a];
                    a += 1;
                } else {
                    *output = rows[b];
                    b += 1;
                }
            }
        }
        std::mem::swap(rows, &mut buffer);
        width *= 2;
    }
    ensure!(!cancel.load(Ordering::Relaxed), "搜索已取消");
    Ok(())
}
