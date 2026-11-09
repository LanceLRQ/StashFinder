//! Persistent catalogs and partition stores. Synthetic M0 helpers retain a separate schema.
pub mod browse;
pub mod catalog;
pub mod file_operations;
pub mod initial;
pub mod search;
pub mod tasks;
pub mod volume;

use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags, params, params_from_iter};
use std::path::Path;

pub fn create_synthetic(path: &Path, count: usize) -> Result<Connection> {
    ensure!(
        count > 0 && count <= 10_000_000,
        "row count must be 1..=10000000"
    );
    // Reserve the path atomically; never replace an existing catalog or benchmark.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let mut db = Connection::open(path)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        PRAGMA foreign_keys=ON; PRAGMA cache_size=-32768;
        CREATE TABLE directories (id INTEGER PRIMARY KEY, volume INTEGER NOT NULL, path_fold TEXT NOT NULL);
        CREATE TABLE entries (id INTEGER PRIMARY KEY, parent INTEGER NOT NULL REFERENCES directories(id),
            name_fold TEXT NOT NULL, size INTEGER, created_utc INTEGER, modified_utc INTEGER);
        PRAGMA user_version=1;")?;
    let tx = db.transaction()?;
    {
        let mut insert = tx.prepare("INSERT INTO directories VALUES (?1, ?2, ?3)")?;
        for id in 0..1200usize {
            insert.execute(params![
                id as i64,
                (id % 12) as i64,
                format!(r"volume-{}\资料\aaa\projects\group-{id:04}\", id % 12)
            ])?;
        }
    }
    tx.commit()?;
    for batch in (0..count).step_by(50_000) {
        let tx = db.transaction()?;
        {
            let mut insert = tx.prepare("INSERT INTO entries VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
            for id in batch..(batch + 50_000).min(count) {
                let name = if id % 10_000 == 0 {
                    format!("rare-视频-{id:08}.mkv")
                } else {
                    format!("aaacccbbb-{:08}.txt", id % 100_000)
                };
                let size = if id % 100 == 0 {
                    None
                } else {
                    Some((id % 65_536) as i64 * 1024)
                };
                insert.execute(params![
                    id as i64,
                    (id % 1200) as i64,
                    name,
                    size,
                    1_700_000_000_i64,
                    1_700_000_000_i64 + id as i64 % 86400
                ])?;
            }
        }
        tx.commit()?;
    }
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(db)
}

pub fn reopen(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}

/// Literal whitespace-separated AND terms for baseline measurement, not the full parser.
pub fn search_sql(db: &Connection, query: &str, limit: usize) -> Result<Vec<i64>> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let clauses: Vec<&str> = terms
        .iter()
        .map(|_| "instr(d.path_fold || e.name_fold, ?) > 0")
        .collect();
    let filter = if clauses.is_empty() {
        "1".to_owned()
    } else {
        clauses.join(" AND ")
    };
    let sql = format!(
        "SELECT e.id FROM entries e JOIN directories d ON d.id=e.parent WHERE {filter} ORDER BY e.id LIMIT ?"
    );
    let mut values: Vec<rusqlite::types::Value> = terms.into_iter().map(Into::into).collect();
    values.push((i64::try_from(limit)?).into());
    Ok(db
        .prepare(&sql)?
        .query_map(params_from_iter(values), |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

pub struct Cache {
    directories: Vec<String>,
    entries: Vec<Entry>,
}
struct Entry {
    id: i64,
    parent: usize,
    name: String,
}

impl Cache {
    pub fn load(db: &Connection) -> Result<Self> {
        let directories = db
            .prepare("SELECT path_fold FROM directories ORDER BY id")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        let count = usize::try_from(
            db.query_row("SELECT count(*) FROM entries", [], |r| r.get::<_, i64>(0))?,
        )?;
        let mut entries = Vec::with_capacity(count);
        let mut statement = db.prepare("SELECT id, parent, name_fold FROM entries ORDER BY id")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let parent = usize::try_from(row.get::<_, i64>(1)?)?;
            ensure!(parent < directories.len(), "invalid synthetic parent");
            entries.push(Entry {
                id: row.get(0)?,
                parent,
                name: row.get(2)?,
            });
        }
        Ok(Self {
            directories,
            entries,
        })
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<i64> {
        if limit == 0 {
            return Vec::new();
        }
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let remaining: Vec<Vec<&str>> = self
            .directories
            .iter()
            .map(|directory| {
                terms
                    .iter()
                    .filter(|term| !directory.contains(term.as_str()))
                    .map(String::as_str)
                    .collect()
            })
            .collect();
        let spans_boundary = terms.iter().any(|term| term.contains('\\'));
        self.entries
            .iter()
            .filter(|entry| {
                if spans_boundary {
                    let path = format!("{}{}", self.directories[entry.parent], entry.name);
                    terms.iter().all(|term| path.contains(term))
                } else {
                    remaining[entry.parent]
                        .iter()
                        .all(|term| entry.name.contains(term))
                }
            })
            .take(limit)
            .map(|entry| entry.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_nullable_metadata_and_matches_sql_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("synthetic.sqlite");
        let db = create_synthetic(&path, 3000).unwrap();
        assert_eq!(
            db.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "wal"
        );
        assert_eq!(
            db.query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        let columns: Vec<String> = db
            .prepare("PRAGMA table_info(entries)")
            .unwrap()
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            columns,
            [
                "id",
                "parent",
                "name_fold",
                "size",
                "created_utc",
                "modified_utc"
            ]
        );
        assert_eq!(
            db.query_row("SELECT size FROM entries WHERE id=0", [], |r| r
                .get::<_, Option<i64>>(0))
                .unwrap(),
            None
        );
        drop(db);
        let db = reopen(&path).unwrap();
        let cache = Cache::load(&db).unwrap();
        for query in [
            "aaa",
            "aaa bbb",
            "BBB AAA",
            "视频",
            "group-0000\\rare",
            "' OR 1=1 --",
            "missing",
            "",
        ] {
            assert_eq!(
                cache.search(query, 200),
                search_sql(&db, query, 200).unwrap(),
                "query: {query}"
            );
        }
        assert_eq!(cache.search("aaa bbb", 1), [1]);
        assert!(cache.search("aaa", 0).is_empty());
        assert!(create_synthetic(&path, 1).is_err());
        assert_eq!(
            db.query_row("SELECT count(*) FROM entries", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3000
        );
    }
}
