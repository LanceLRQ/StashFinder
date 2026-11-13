use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{path::Path, time::Duration};

pub struct Volume {
    pub(crate) db: Connection,
}

#[derive(Clone)]
pub struct Object {
    pub identity: Option<stashfinder_core::file_operations::NativeIdentity>,
    pub id: Vec<u8>,
    pub source: String,
    pub kind: String,
    pub size: Option<i64>,
    pub created: Option<[u8; 8]>,
    pub modified: Option<[u8; 8]>,
    pub attributes: u32,
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

pub struct Changes<'a> {
    pub objects: &'a [Object],
    pub entries: &'a [Entry],
    pub removed: &'a [Vec<u8>],
    pub pending: &'a [Vec<u8>],
    pub cursor: Option<&'a Cursor>,
    pub replace_links: bool,
}

pub(crate) fn configure(db: &Connection) -> Result<()> {
    db.busy_timeout(Duration::from_secs(2))?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA cache_size=-65536;")?;
    Ok(())
}

pub(crate) fn read_native_identity(
    db: &Connection,
    generation: i64,
    object: &[u8],
) -> Result<Option<stashfinder_core::file_operations::NativeIdentity>> {
    let bytes: Option<Vec<u8>> = db
        .query_row(
            "SELECT native_id FROM native_identity WHERE generation=?1 AND object=?2",
            params![generation, object],
            |r| r.get(0),
        )
        .optional()?;
    bytes
        .map(|b| {
            ensure!(b.len() == 12, "原生身份数据损坏");
            Ok(stashfinder_core::file_operations::NativeIdentity {
                volume: u32::from_le_bytes(b[..4].try_into()?),
                file: u64::from_le_bytes(b[4..].try_into()?),
            })
        })
        .transpose()
}

impl Volume {
    pub fn inspect_manifest(path: &Path) -> Result<String> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        ensure!(
            matches!(
                db.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))?,
                1..=5
            ),
            "不支持的分区库版本"
        );
        ensure!(
            db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))? == "ok",
            "分区库损坏"
        );
        Ok(db.query_row("SELECT value FROM manifest WHERE id=1", [], |r| r.get(0))?)
    }
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
        let mut volume = Self { db };
        volume.migrate()?;
        Ok(volume)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        Self::open_connection(db, true)
    }

    pub(crate) fn open_connection(db: Connection, check: bool) -> Result<Self> {
        let version = db.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))?;
        ensure!(matches!(version, 1..=5), "不支持的分区库版本");
        if check || version < 5 {
            ensure!(
                db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))? == "ok",
                "分区库损坏"
            );
        }
        configure(&db)?;
        let mut volume = Self { db };
        if version < 5 {
            volume.migrate()?;
        }
        Ok(volume)
    }

    fn migrate(&mut self) -> Result<()> {
        if self
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))?
            == 1
        {
            let tx = self.db.transaction()?;
            tx.execute_batch("ALTER TABLE objects ADD COLUMN attributes INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE generations ADD COLUMN phase TEXT NOT NULL DEFAULT 'building';
                ALTER TABLE generations ADD COLUMN started INTEGER;
                ALTER TABLE generations ADD COLUMN completed INTEGER;
                ALTER TABLE generations ADD COLUMN error TEXT;
                UPDATE generations SET phase='ready' WHERE id=(SELECT active_generation FROM index_state WHERE id=1);
                CREATE INDEX paths_parent ON paths(generation,parent);
                PRAGMA user_version=2;")?;
            tx.commit()?;
        }
        if self
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))?
            == 2
        {
            self.db.execute_batch("BEGIN IMMEDIATE; ALTER TABLE generations ADD COLUMN revision INTEGER NOT NULL DEFAULT 0; PRAGMA user_version=3; COMMIT;")?;
        }
        if self
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))?
            == 3
        {
            let tx = self.db.transaction()?;
            tx.execute_batch("CREATE TABLE native_identity (generation INTEGER NOT NULL, object BLOB NOT NULL, identity_kind TEXT NOT NULL CHECK(identity_kind='file_id64'), native_id BLOB NOT NULL CHECK(length(native_id)=12), PRIMARY KEY(generation,object), FOREIGN KEY(generation,object) REFERENCES objects(generation,id) ON DELETE CASCADE); PRAGMA user_version=4;")?;
            tx.commit()?;
        }
        if self
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))?
            == 4
        {
            let tx = self.db.transaction()?;
            tx.execute_batch("CREATE INDEX IF NOT EXISTS paths_object ON paths(generation,object); PRAGMA user_version=5;")?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn native_identity(
        &self,
        generation: i64,
        object: &[u8],
    ) -> Result<Option<stashfinder_core::file_operations::NativeIdentity>> {
        read_native_identity(&self.db, generation, object)
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
        self.db.execute(
            "INSERT INTO generations(started) VALUES(?1)",
            [stashfinder_core::assets::now() as i64],
        )?;
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
        self.apply_changes(
            generation,
            Changes {
                objects,
                entries,
                removed: &[],
                pending,
                cursor,
                replace_links: false,
            },
        )
    }

    pub fn apply_changes(&mut self, generation: i64, changes: Changes<'_>) -> Result<()> {
        self.commit_changes(generation, changes, false)
    }

    pub fn apply_incremental(&mut self, generation: i64, changes: Changes<'_>) -> Result<()> {
        ensure!(changes.cursor.is_some(), "增量事务缺少游标");
        self.commit_changes(generation, changes, true)
    }

    fn commit_changes(
        &mut self,
        generation: i64,
        changes: Changes<'_>,
        incremental: bool,
    ) -> Result<()> {
        let Changes {
            objects,
            entries,
            removed,
            pending,
            cursor,
            replace_links,
        } = changes;
        // Acquire the writer before reading so a capacity update cannot invalidate the snapshot.
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let writable: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM generations WHERE id=?1) AND (EXISTS(SELECT 1 FROM index_state WHERE active_generation=?1)=?2)",
            params![generation, incremental], |r| r.get(0),
        )?;
        ensure!(writable, "索引代次不存在或已经发布");
        tx.execute(
            "UPDATE generations SET complete=0,revision=revision+1 WHERE id=?1",
            [generation],
        )?;
        for id in removed {
            // Keep the small recursive frontier outermost so each lookup uses paths_parent.
            tx.execute("WITH RECURSIVE doomed(id) AS (SELECT id FROM paths WHERE generation=?1 AND object=?2 UNION SELECT p.id FROM doomed d CROSS JOIN paths p ON p.parent=d.id WHERE p.generation=?1) DELETE FROM paths WHERE generation=?1 AND id IN (SELECT id FROM doomed)",params![generation,id])?;
            tx.execute(
                "DELETE FROM pending_metadata WHERE generation=?1 AND object=?2",
                params![generation, id],
            )?;
            tx.execute(
                "DELETE FROM objects WHERE generation=?1 AND id=?2",
                params![generation, id],
            )?;
        }
        // Native IDs are little-endian bytes, so source order scatters writes
        // across the B-tree. Stable sorting also preserves the last observation
        // when a batch contains several hard links to the same object.
        let mut objects = objects.iter().collect::<Vec<_>>();
        objects.sort_by(|a, b| a.id.cmp(&b.id));
        for object in objects {
            if replace_links {
                tx.prepare_cached("DELETE FROM paths WHERE generation=?1 AND object=?2")?
                    .execute(params![generation, object.id])?;
            }
            ensure!(!object.id.is_empty(), "文件 ID 为空");
            tx.prepare_cached("INSERT INTO objects VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(generation,id) DO UPDATE SET source=excluded.source,kind=excluded.kind,size=excluded.size,created=excluded.created,modified=excluded.modified,attributes=excluded.attributes")?.execute(params![generation, object.id, object.source, object.kind, object.size, object.created.as_ref().map(|v|v.as_slice()), object.modified.as_ref().map(|v|v.as_slice()), object.attributes])?;
            tx.prepare_cached("DELETE FROM native_identity WHERE generation=?1 AND object=?2")?
                .execute(params![generation, object.id])?;
            if let Some(identity) = &object.identity {
                let bytes = [
                    identity.volume.to_le_bytes().as_slice(),
                    identity.file.to_le_bytes().as_slice(),
                ]
                .concat();
                tx.prepare_cached("INSERT INTO native_identity VALUES(?1,?2,'file_id64',?3)")?
                    .execute(params![generation, object.id, bytes])?;
            }
            // Replaced metadata and its pending state share the cursor transaction.
            tx.prepare_cached("DELETE FROM pending_metadata WHERE generation=?1 AND object=?2")?
                .execute(params![generation, object.id])?;
        }
        let mut entries = entries.iter().collect::<Vec<_>>();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        for entry in entries {
            let name: Vec<u8> = entry.name.iter().flat_map(|c| c.to_le_bytes()).collect();
            ensure!(!name.is_empty(), "路径名称为空");
            tx.prepare_cached("INSERT INTO paths VALUES(?1,?2,?3,?4,?5) ON CONFLICT(generation,id) DO UPDATE SET object=excluded.object,parent=excluded.parent,name=excluded.name")?.execute(params![generation, entry.id, entry.object, entry.parent, name])?;
        }
        let mut pending = pending.iter().collect::<Vec<_>>();
        pending.sort();
        for id in pending {
            tx.prepare_cached("INSERT OR IGNORE INTO pending_metadata VALUES(?1,?2)")?
                .execute(params![generation, id])?;
        }
        if let Some(cursor) = cursor {
            ensure!(cursor.next >= 0, "USN 游标无效");
            if incremental {
                let valid: bool = tx.query_row(
                    "SELECT journal=?2 AND next_usn<=?3 FROM generations WHERE id=?1",
                    params![generation, cursor.journal, cursor.next],
                    |r| r.get(0),
                )?;
                ensure!(valid, "增量日志或游标不连续");
            }
            ensure!(
                tx.execute(
                    "UPDATE generations SET journal=?1,next_usn=?2 WHERE id=?3",
                    params![cursor.journal, cursor.next, generation]
                )? == 1,
                "索引代次不存在"
            );
        }
        if incremental {
            tx.execute(
                "UPDATE generations SET phase='syncing',error=NULL WHERE id=?1",
                [generation],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn mark_stale(&self, generation: i64, message: &str) -> Result<()> {
        self.db.execute(
            "UPDATE generations SET phase='needs_validation',error=?2 WHERE id=?1",
            params![generation, message],
        )?;
        Ok(())
    }

    pub fn finish_incremental(&mut self, generation: i64) -> Result<()> {
        ensure!(
            self.active_generation()? == Some(generation),
            "活动代次已变化"
        );
        self.complete(generation)?;
        self.publish_mode(generation, "ready", true)
    }

    pub fn complete(&self, generation: i64) -> Result<()> {
        ensure!(
            self.db.execute(
                "UPDATE generations SET complete=1 WHERE id=?1 AND phase!='initial_paths'",
                [generation]
            )? == 1,
            "索引代次不存在或初始路径尚未导入"
        );
        Ok(())
    }

    pub fn publish(&mut self, generation: i64) -> Result<()> {
        self.publish_mode(generation, "ready", false)
    }

    pub fn publish_snapshot(&mut self, generation: i64) -> Result<()> {
        self.publish_mode(generation, "snapshot", false)
    }

    fn publish_mode(&mut self, generation: i64, phase: &str, incremental: bool) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let complete: bool = tx.query_row(
            "SELECT complete FROM generations WHERE id=?1",
            [generation],
            |r| r.get(0),
        )?;
        let pending = !incremental
            && tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pending_metadata WHERE generation=?1)",
                [generation],
                |r| r.get::<_, bool>(0),
            )?;
        ensure!(complete && !pending, "索引尚未完成或仍有元数据待补齐");
        tx.execute(
            "UPDATE index_state SET active_generation=?1 WHERE id=1",
            [generation],
        )?;
        tx.execute(
            "UPDATE generations SET phase=?3,completed=?2,error=NULL,revision=revision+1 WHERE id=?1",
            params![generation, stashfinder_core::assets::now() as i64, phase],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn has_pending_metadata(&self, generation: i64) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM pending_metadata WHERE generation=?1)",
            [generation],
            |r| r.get(0),
        )?)
    }

    pub fn pending_metadata(&self, generation: i64) -> Result<Vec<u64>> {
        let mut stmt = self
            .db
            .prepare("SELECT object FROM pending_metadata WHERE generation=?1 ORDER BY object")?;
        stmt.query_map([generation], |row| row.get::<_, [u8; 8]>(0))?
            .map(|row| Ok(u64::from_le_bytes(row?)))
            .collect()
    }

    pub fn has_path(&self, generation: i64, id: &[u8]) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM paths WHERE generation=?1 AND id=?2)",
            params![generation, id],
            |r| r.get(0),
        )?)
    }

    pub fn exclude_subtrees(&mut self, generation: i64, roots: &[Vec<u8>]) -> Result<bool> {
        let mut present = Vec::new();
        for root in roots {
            if self.has_path(generation, root)? {
                present.push(root);
            }
        }
        if present.is_empty() {
            return Ok(false);
        }
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for root in present {
            let is_root: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM paths WHERE generation=?1 AND id=?2 AND parent IS NULL)",
                params![generation, root],
                |r| r.get(0),
            )?;
            ensure!(!is_root, "不能排除整个分区");
            tx.execute("WITH RECURSIVE tree(id) AS (SELECT id FROM paths WHERE generation=?1 AND id=?2 UNION SELECT p.id FROM tree t CROSS JOIN paths p ON p.parent=t.id WHERE p.generation=?1) DELETE FROM paths WHERE generation=?1 AND id IN (SELECT id FROM tree)", params![generation,root])?;
        }
        tx.execute("DELETE FROM pending_metadata WHERE generation=?1 AND object NOT IN (SELECT object FROM paths WHERE generation=?1)", [generation])?;
        tx.execute("DELETE FROM objects WHERE generation=?1 AND id NOT IN (SELECT object FROM paths WHERE generation=?1)", [generation])?;
        tx.execute(
            "UPDATE generations SET revision=revision+1 WHERE id=?1",
            [generation],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn has_object(&self, generation: i64, id: &[u8]) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM objects WHERE generation=?1 AND id=?2)",
            params![generation, id],
            |row| row.get(0),
        )?)
    }

    pub fn prune_inventory(&mut self, generation: i64) -> Result<()> {
        ensure!(
            self.active_generation()? != Some(generation),
            "不能修改已发布代次"
        );
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM pending_metadata WHERE generation=?1 AND object NOT IN (SELECT object FROM paths WHERE generation=?1)",[generation])?;
        tx.execute("DELETE FROM objects WHERE generation=?1 AND id NOT IN (SELECT object FROM paths WHERE generation=?1)",[generation])?;
        tx.commit()?;
        Ok(())
    }

    pub fn fail_generation(&self, generation: i64, message: &str) -> Result<()> {
        ensure!(self.db.execute("UPDATE generations SET phase='failed',complete=0,error=?2 WHERE id=?1 AND id NOT IN (SELECT active_generation FROM index_state WHERE active_generation IS NOT NULL)",params![generation,message])? == 1,"构建代次不存在或已经发布");
        Ok(())
    }

    pub fn index_status(&self) -> Result<stashfinder_core::index::IndexStatus> {
        let active = self.active_generation()?;
        let row = self
            .db
            .query_row(
                "SELECT id,phase,started,completed,error FROM generations ORDER BY id DESC LIMIT 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((generation, phase, started, completed, error)) = row else {
            return Ok(stashfinder_core::index::IndexStatus {
                phase: "not_indexed".into(),
                ..Default::default()
            });
        };
        let records = self.db.query_row(
            "SELECT count(*) FROM paths WHERE generation=?1 AND parent IS NOT NULL",
            [generation],
            |r| r.get::<_, i64>(0),
        )?;
        Ok(stashfinder_core::index::IndexStatus {
            task_id: None,
            journal_missing: false,
            generation: Some(generation),
            active_generation: active,
            phase,
            started: started.map(u64::try_from).transpose()?,
            completed: completed.map(u64::try_from).transpose()?,
            records: records.try_into()?,
            error,
        })
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
