use crate::volume::{Volume, configure};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use stashfinder_core::assets::{IdentityMatch, Observation, Registration, now, validate_name};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct Catalog {
    db: Connection,
    root: PathBuf,
}

impl Catalog {
    pub fn read(root: &Path) -> Result<Self> {
        ensure!(root.is_absolute(), "数据库目录必须为绝对路径");
        let db = Connection::open_with_flags(
            root.join("catalog.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        ensure!(
            matches!(
                db.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))?,
                1 | 2
            ),
            "资产库版本不支持"
        );
        Ok(Self {
            db,
            root: root.into(),
        })
    }
    pub fn open(root: &Path) -> Result<Self> {
        ensure!(root.is_absolute(), "数据库目录必须为绝对路径");
        std::fs::create_dir_all(root.join("volumes"))?;
        let path = root.join("catalog.sqlite");
        let fresh = !path.exists();
        if fresh {
            ensure!(
                std::fs::read_dir(root.join("volumes"))?.next().is_none(),
                "资产库缺失但仍有分区库，需要恢复，未创建空库"
            );
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
        }
        let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        if fresh {
            configure(&db)?;
            db.execute_batch("BEGIN;
                CREATE TABLE disks (id TEXT PRIMARY KEY, identity_key TEXT UNIQUE);
                CREATE TABLE partitions (id TEXT PRIMARY KEY, disk_id TEXT NOT NULL REFERENCES disks(id), name TEXT NOT NULL, revision INTEGER NOT NULL);
                CREATE TABLE filesystems (id TEXT PRIMARY KEY, partition_id TEXT NOT NULL UNIQUE REFERENCES partitions(id), state TEXT NOT NULL CHECK(state IN ('preparing','ready')), observation TEXT NOT NULL, last_seen INTEGER);
                PRAGMA user_version=1; COMMIT;")?;
        } else {
            ensure!(
                matches!(
                    db.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))?,
                    1 | 2
                ),
                "资产库版本不支持；需要恢复，未创建空库"
            );
            ensure!(
                db.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))? == "ok",
                "资产库损坏，需要恢复"
            );
            configure(&db)?;
        }
        if db.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))? == 1 {
            db.execute_batch("BEGIN; CREATE TABLE historical(partition_id TEXT PRIMARY KEY REFERENCES partitions(id)); PRAGMA user_version=2; COMMIT;")?;
        }
        let mut catalog = Self {
            db,
            root: root.into(),
        };
        for row in catalog.list()? {
            if row.state == "preparing" {
                // A broken child remains preparing and is surfaced through health().
                let _ = catalog.finish(&row.partition_id);
            } else {
                let _ = catalog.sync_manifest(&row);
            }
        }
        Ok(catalog)
    }

    pub fn list(&self) -> Result<Vec<Registration>> {
        let version: i64 = self.db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let state = if version >= 2 {
            "CASE WHEN EXISTS(SELECT 1 FROM historical h WHERE h.partition_id=p.id) THEN 'historical' ELSE f.state END"
        } else {
            "f.state"
        };
        let mut stmt = self.db.prepare(&format!("SELECT p.disk_id,p.id,f.id,p.name,p.revision,{state},f.observation,f.last_seen FROM partitions p JOIN filesystems f ON f.partition_id=p.id ORDER BY p.rowid"))?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, Option<i64>>(7)?,
            ))
        })?;
        rows.map(|row| {
            let (
                disk_id,
                partition_id,
                filesystem_id,
                name,
                revision,
                state,
                observation,
                last_seen,
            ) = row?;
            Ok(Registration {
                disk_id,
                partition_id,
                filesystem_id,
                name,
                revision: revision.try_into()?,
                state,
                observation: serde_json::from_str(&observation)?,
                last_seen: last_seen.map(u64::try_from).transpose()?,
            })
        })
        .collect()
    }

    pub fn get(&self, partition: &str) -> Result<Registration> {
        self.list()?
            .into_iter()
            .find(|r| r.partition_id == partition)
            .context("登记不存在")
    }

    pub fn prepare(&mut self, observation: &Observation, name: &str) -> Result<Registration> {
        let name = validate_name(name)?;
        ensure!(observation.eligible(), "分区不可登记");
        if let Some(existing) = self.list()?.into_iter().find(|r| {
            r.state != "historical"
                && r.observation.identity.compare(&observation.identity) == IdentityMatch::Same
        }) {
            return Ok(existing);
        }
        let disk_key = observation.identity.disk_key();
        let existing_disk: Option<String> = self
            .db
            .query_row(
                "SELECT id FROM disks WHERE identity_key=?1",
                [&disk_key],
                |r| r.get(0),
            )
            .optional()?;
        let registration = Registration {
            disk_id: existing_disk.unwrap_or_else(|| Uuid::new_v4().to_string()),
            partition_id: Uuid::new_v4().to_string(),
            filesystem_id: Uuid::new_v4().to_string(),
            name,
            revision: 1,
            state: "preparing".into(),
            observation: observation.clone(),
            last_seen: Some(now()),
        };
        self.insert(&registration)?;
        Ok(registration)
    }

    fn insert(&mut self, row: &Registration) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO disks VALUES(?1,?2)",
            params![row.disk_id, row.observation.identity.disk_key()],
        )?;
        tx.execute(
            "INSERT INTO partitions VALUES(?1,?2,?3,?4)",
            params![
                row.partition_id,
                row.disk_id,
                row.name,
                i64::try_from(row.revision)?
            ],
        )?;
        tx.execute(
            "INSERT INTO filesystems VALUES(?1,?2,?3,?4,?5)",
            params![
                row.filesystem_id,
                row.partition_id,
                if row.state == "historical" {
                    "ready"
                } else {
                    &row.state
                },
                serde_json::to_string(&row.observation)?,
                row.last_seen.map(i64::try_from).transpose()?
            ],
        )?;
        if row.state == "historical" {
            tx.execute("INSERT INTO historical VALUES(?1)", [&row.partition_id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn volume_path(&self, filesystem: &str) -> Result<PathBuf> {
        let id = Uuid::parse_str(filesystem)?;
        Ok(self.root.join("volumes").join(format!("{id}.sqlite")))
    }

    pub fn finish(&mut self, partition: &str) -> Result<()> {
        let mut row = self.get(partition)?;
        if row.state == "ready" {
            return self.health(&row.filesystem_id);
        }
        let path = self.volume_path(&row.filesystem_id)?;
        row.state = "ready".into();
        let child = if path.exists() {
            self.open_owned(&row)?
        } else {
            Volume::create(&path, &serde_json::to_string(&row)?)?
        };
        if let Some(capacity) = &row.observation.capacity {
            child.set_capacity(capacity)?;
        }
        child.set_manifest(&serde_json::to_string(&row)?)?;
        self.db.execute(
            "UPDATE filesystems SET state='ready' WHERE id=?1",
            [&row.filesystem_id],
        )?;
        Ok(())
    }

    pub fn retry(&mut self, partition: &str, observation: &Observation) -> Result<()> {
        let row = self.get(partition)?;
        ensure!(row.state == "preparing", "已完成登记不能重新初始化");
        ensure!(
            row.observation.identity.compare(&observation.identity) == IdentityMatch::Same,
            "身份未确认，不能重试"
        );
        let new_id = Uuid::new_v4().to_string();
        self.db.execute(
            "UPDATE filesystems SET id=?1 WHERE partition_id=?2",
            params![new_id, partition],
        )?;
        self.finish(partition)
    }

    pub fn health(&self, filesystem: &str) -> Result<()> {
        let row = self
            .list()?
            .into_iter()
            .find(|r| r.filesystem_id == filesystem)
            .context("登记不存在")?;
        ensure!(
            matches!(row.state.as_str(), "ready" | "historical"),
            "登记初始化未完成，可重试"
        );
        let child = self.open_owned(&row)?;
        let saved: Registration = serde_json::from_str(&child.manifest()?)?;
        ensure!(
            saved.revision == row.revision && saved.name == row.name,
            "名称恢复副本尚未同步，请刷新重试"
        );
        Ok(())
    }

    fn open_owned(&self, row: &Registration) -> Result<Volume> {
        let path = self.volume_path(&row.filesystem_id)?;
        let saved: Registration = serde_json::from_str(&Volume::inspect_manifest(&path)?)?;
        ensure!(
            saved.filesystem_id == row.filesystem_id
                && saved.partition_id == row.partition_id
                && saved.disk_id == row.disk_id
                && saved.observation.identity == row.observation.identity,
            "分区库归属不一致"
        );
        Volume::open(&path)
    }

    fn sync_manifest(&self, row: &Registration) -> Result<()> {
        let child = self.open_owned(row)?;
        let saved: Registration = serde_json::from_str(&child.manifest()?)?;
        ensure!(
            saved.revision <= row.revision,
            "分区配置版本高于资产库，需要恢复"
        );
        child.set_manifest(&serde_json::to_string(row)?)
    }

    pub fn rename(&mut self, partition: &str, name: &str) -> Result<()> {
        let name = validate_name(name)?;
        ensure!(
            self.db.execute(
                "UPDATE partitions SET name=?1,revision=revision+1 WHERE id=?2",
                params![name, partition]
            )? == 1,
            "登记不存在"
        );
        self.sync_manifest(&self.get(partition)?)
            .context("名称已保存，但恢复副本尚未同步")
    }

    pub fn observe(&mut self, partition: &str, observation: &Observation) -> Result<()> {
        let mut row = self.get(partition)?;
        ensure!(row.state == "ready", "历史记录不能自动更新");
        self.health(&row.filesystem_id)?;
        ensure!(
            row.observation.identity.compare(&observation.identity) == IdentityMatch::Same,
            "分区身份未确认"
        );
        let child = Volume::open(&self.volume_path(&row.filesystem_id)?)?;
        if let Some(capacity) = &observation.capacity {
            child.set_capacity(capacity)?;
        }
        let capacity = child.capacity()?;
        row.observation = observation.clone();
        row.observation.capacity = capacity;
        row.last_seen = Some(now());
        self.db.execute(
            "UPDATE filesystems SET observation=?1,last_seen=?2 WHERE id=?3",
            params![
                serde_json::to_string(&row.observation)?,
                row.last_seen.map(i64::try_from).transpose()?,
                row.filesystem_id
            ],
        )?;
        self.sync_manifest(&row)
    }

    pub fn archive(&mut self, partition: &str) -> Result<()> {
        let row = self.get(partition)?;
        ensure!(row.state != "preparing", "未完成登记不能归档");
        self.db
            .execute("INSERT OR IGNORE INTO historical VALUES(?1)", [partition])?;
        self.sync_manifest(&self.get(partition)?)
    }

    pub fn recover(source: &Path, destination: &Path) -> Result<Vec<String>> {
        ensure!(!destination.exists(), "恢复目录必须不存在");
        let mut candidate = Self::open(destination)?;
        let mut errors = Vec::new();
        let mut recovered = Vec::new();
        for entry in std::fs::read_dir(source.join("volumes"))? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("sqlite") {
                continue;
            }
            let result = (|| -> Result<Registration> {
                let mut row: Registration =
                    serde_json::from_str(&Volume::inspect_manifest(&path)?)?;
                ensure!(
                    matches!(row.state.as_str(), "ready" | "historical"),
                    "未发布的库不用于恢复"
                );
                ensure!(
                    path.file_stem().and_then(|s| s.to_str()) == Some(row.filesystem_id.as_str()),
                    "库文件名和身份不一致"
                );
                row.last_seen = None;
                Ok(row)
            })();
            match result {
                Ok(row) => recovered.push((path, row)),
                Err(error) => errors.push(format!("{}: {error:#}", path.display())),
            }
        }
        for (path, row) in &recovered {
            if recovered
                .iter()
                .filter(|(_, other)| {
                    other.partition_id == row.partition_id
                        || (other.state != "historical"
                            && row.state != "historical"
                            && other
                                .observation
                                .identity
                                .compare(&row.observation.identity)
                                == IdentityMatch::Same)
                })
                .count()
                > 1
            {
                errors.push(format!(
                    "{}: 恢复副本归属冲突，需要人工确认",
                    path.display()
                ));
                continue;
            }
            // The recovery catalog is a candidate only; child data stays in the original directory.
            if let Err(error) = candidate.insert(row) {
                errors.push(format!("{}: {error:#}", path.display()));
            }
        }
        Ok(errors)
    }
}
