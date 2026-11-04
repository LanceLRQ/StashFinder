use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{path::Path, time::Duration};

pub struct Volume {
    db: Connection,
}

#[derive(Clone)]
pub struct Object {
    pub id: Vec<u8>,
    pub source: String,
    pub kind: String,
    pub size: Option<i64>,
    pub created: Option<[u8; 8]>,
    pub modified: Option<[u8; 8]>,
}
pub struct Entry {
    pub id: Vec<u8>,
    pub object: Vec<u8>,
    pub parent: Option<Vec<u8>>,
    pub name: Vec<u16>,
}
pub struct Cursor {
    pub journal: Vec<u8>,
    pub next: i64,
}

pub(crate) fn configure(db: &Connection) -> Result<()> {
    db.busy_timeout(Duration::from_secs(2))?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
    Ok(())
}

impl Volume {
    pub fn create(path: &Path, manifest: &str) -> Result<Self> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let mut db = Connection::open(path)?;
        configure(&db)?;
        let tx = db.transaction()?;
        tx.execute_batch("CREATE TABLE manifest (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);
            CREATE TABLE generations (id INTEGER PRIMARY KEY, complete INTEGER NOT NULL DEFAULT 0 CHECK(complete IN (0,1)), journal BLOB, next_usn INTEGER);
            CREATE TABLE index_state (id INTEGER PRIMARY KEY CHECK(id=1), active_generation INTEGER REFERENCES generations(id));
            INSERT INTO index_state VALUES(1,NULL);
            CREATE TABLE capacity_snapshot (id INTEGER PRIMARY KEY CHECK(id=1), total INTEGER NOT NULL, free INTEGER NOT NULL CHECK(free>=0 AND free<=total), sampled_at INTEGER NOT NULL);
            CREATE TABLE objects (generation INTEGER NOT NULL REFERENCES generations(id), id BLOB NOT NULL, source TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('file','folder')), size INTEGER CHECK(size>=0), created BLOB, modified BLOB, PRIMARY KEY(generation,id));
            CREATE TABLE paths (generation INTEGER NOT NULL, id BLOB NOT NULL, object BLOB NOT NULL, parent BLOB, name BLOB NOT NULL, PRIMARY KEY(generation,id), FOREIGN KEY(generation,object) REFERENCES objects(generation,id), FOREIGN KEY(generation,parent) REFERENCES paths(generation,id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TABLE pending_metadata (generation INTEGER NOT NULL, object BLOB NOT NULL, PRIMARY KEY(generation,object), FOREIGN KEY(generation,object) REFERENCES objects(generation,id));
            PRAGMA user_version=1;")?;
        tx.execute("INSERT INTO manifest VALUES(1,?1)", [manifest])?;
        tx.commit()?;
        Ok(Self { db })
    }

    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        ensure!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))? == 1,
            "不支持的分区库版本"
        );
        ensure!(
            db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))? == "ok",
            "分区库损坏"
        );
        configure(&db)?;
        Ok(Self { db })
    }

    pub fn manifest(&self) -> Result<String> {
        Ok(self
            .db
            .query_row("SELECT value FROM manifest WHERE id=1", [], |r| r.get(0))?)
    }

    pub fn set_manifest(&self, value: &str) -> Result<()> {
        self.db
            .execute("UPDATE manifest SET value=?1 WHERE id=1", [value])?;
        Ok(())
    }

    pub fn capacity(&self) -> Result<Option<stashfinder_core::assets::Capacity>> {
        let row = self
            .db
            .query_row(
                "SELECT total,free,sampled_at FROM capacity_snapshot WHERE id=1",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(total, free, sampled_at)| {
            Ok(stashfinder_core::assets::Capacity {
                total: total.try_into()?,
                free: free.try_into()?,
                sampled_at: sampled_at.try_into()?,
            })
        })
        .transpose()
    }

    pub fn set_capacity(&self, capacity: &stashfinder_core::assets::Capacity) -> Result<()> {
        ensure!(capacity.free <= capacity.total, "容量快照无效");
        self.db.execute("INSERT INTO capacity_snapshot VALUES(1,?1,?2,?3) ON CONFLICT(id) DO UPDATE SET total=excluded.total,free=excluded.free,sampled_at=excluded.sampled_at", params![i64::try_from(capacity.total)?, i64::try_from(capacity.free)?, i64::try_from(capacity.sampled_at)?])?;
        Ok(())
    }

    pub fn begin_generation(&mut self) -> Result<i64> {
        self.db
            .execute("INSERT INTO generations DEFAULT VALUES", [])?;
        Ok(self.db.last_insert_rowid())
    }

    pub fn write_batch(
        &mut self,
        generation: i64,
        objects: &[Object],
        entries: &[Entry],
        pending: &[Vec<u8>],
        cursor: Option<&Cursor>,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        let writable: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM generations WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM index_state WHERE active_generation=?1)",
            [generation], |r| r.get(0),
        )?;
        ensure!(writable, "索引代次不存在或已经发布");
        tx.execute(
            "UPDATE generations SET complete=0 WHERE id=?1",
            [generation],
        )?;
        for object in objects {
            ensure!(!object.id.is_empty(), "文件 ID 为空");
            tx.execute("INSERT INTO objects VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(generation,id) DO UPDATE SET source=excluded.source,kind=excluded.kind,size=excluded.size,created=excluded.created,modified=excluded.modified", params![generation, object.id, object.source, object.kind, object.size, object.created.as_ref().map(|v|v.as_slice()), object.modified.as_ref().map(|v|v.as_slice())])?;
            // Replaced metadata and its pending state share the cursor transaction.
            tx.execute(
                "DELETE FROM pending_metadata WHERE generation=?1 AND object=?2",
                params![generation, object.id],
            )?;
        }
        for entry in entries {
            let name: Vec<u8> = entry.name.iter().flat_map(|c| c.to_le_bytes()).collect();
            ensure!(!name.is_empty(), "路径名称为空");
            tx.execute("INSERT INTO paths VALUES(?1,?2,?3,?4,?5) ON CONFLICT(generation,id) DO UPDATE SET object=excluded.object,parent=excluded.parent,name=excluded.name", params![generation, entry.id, entry.object, entry.parent, name])?;
        }
        for id in pending {
            tx.execute(
                "INSERT OR IGNORE INTO pending_metadata VALUES(?1,?2)",
                params![generation, id],
            )?;
        }
        if let Some(cursor) = cursor {
            ensure!(cursor.next >= 0, "USN 游标无效");
            ensure!(
                tx.execute(
                    "UPDATE generations SET journal=?1,next_usn=?2 WHERE id=?3",
                    params![cursor.journal, cursor.next, generation]
                )? == 1,
                "索引代次不存在"
            );
        }
        tx.commit()?;
        Ok(())
    }

    pub fn complete(&self, generation: i64) -> Result<()> {
        ensure!(
            self.db.execute(
                "UPDATE generations SET complete=1 WHERE id=?1",
                [generation]
            )? == 1,
            "索引代次不存在"
        );
        Ok(())
    }

    pub fn publish(&mut self, generation: i64) -> Result<()> {
        let tx = self.db.transaction()?;
        let complete: bool = tx.query_row(
            "SELECT complete FROM generations WHERE id=?1",
            [generation],
            |r| r.get(0),
        )?;
        let pending: i64 = tx.query_row(
            "SELECT count(*) FROM pending_metadata WHERE generation=?1",
            [generation],
            |r| r.get(0),
        )?;
        ensure!(complete && pending == 0, "索引尚未完成或仍有元数据待补齐");
        tx.execute(
            "UPDATE index_state SET active_generation=?1 WHERE id=1",
            [generation],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn active_generation(&self) -> Result<Option<i64>> {
        Ok(self.db.query_row(
            "SELECT active_generation FROM index_state WHERE id=1",
            [],
            |r| r.get(0),
        )?)
    }
    pub fn object_count(&self, generation: i64) -> Result<i64> {
        Ok(self.db.query_row(
            "SELECT count(*) FROM objects WHERE generation=?1",
            [generation],
            |r| r.get(0),
        )?)
    }
    pub fn cursor(&self, generation: i64) -> Result<Option<Cursor>> {
        Ok(self
            .db
            .query_row(
                "SELECT journal,next_usn FROM generations WHERE id=?1 AND journal IS NOT NULL",
                [generation],
                |r| {
                    Ok(Cursor {
                        journal: r.get(0)?,
                        next: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }
}
