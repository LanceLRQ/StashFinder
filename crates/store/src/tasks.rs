use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use stashfinder_core::{assets::now, index::IndexMode, tasks::Task};
use std::path::Path;

pub struct Tasks {
    db: Connection,
}

impl Tasks {
    pub fn open(root: &Path) -> Result<Self> {
        let path = root.join("tasks.sqlite");
        let fresh = !path.exists();
        let mut db = Connection::open(path)?;
        crate::volume::configure(&db)?;
        if fresh {
            db.execute_batch("BEGIN;
                CREATE TABLE tasks(seq INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT UNIQUE NOT NULL,partition_id TEXT NOT NULL,kind TEXT NOT NULL,mode TEXT NOT NULL,automatic INTEGER NOT NULL,state TEXT NOT NULL,created INTEGER NOT NULL,updated INTEGER NOT NULL,error TEXT);
                CREATE UNIQUE INDEX tasks_active ON tasks(partition_id) WHERE state IN ('queued','running','waiting');
                CREATE TABLE policy(id INTEGER PRIMARY KEY CHECK(id=1),enabled INTEGER NOT NULL,revision INTEGER NOT NULL);
                INSERT INTO policy VALUES(1,1,1);
                CREATE TABLE paused(partition_id TEXT PRIMARY KEY);
                CREATE TRIGGER task_insert AFTER INSERT ON tasks BEGIN UPDATE policy SET revision=revision+1; END;
                CREATE TRIGGER task_update AFTER UPDATE ON tasks BEGIN UPDATE policy SET revision=revision+1; END;
                CREATE TRIGGER task_delete AFTER DELETE ON tasks BEGIN UPDATE policy SET revision=revision+1; END;
                CREATE TRIGGER pause_insert AFTER INSERT ON paused BEGIN UPDATE policy SET revision=revision+1; END;
                CREATE TRIGGER pause_delete AFTER DELETE ON paused BEGIN UPDATE policy SET revision=revision+1; END;
                PRAGMA user_version=1; COMMIT;")?;
        }
        if db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? == 1 {
            db.execute_batch("BEGIN; CREATE TABLE scan_modes(partition_id TEXT PRIMARY KEY); PRAGMA user_version=2; COMMIT;")?;
        }
        if db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? == 2 {
            let tx = db.transaction()?;
            Self::prune(&tx)?;
            tx.pragma_update(None, "user_version", 3)?;
            tx.commit()?;
        }
        ensure!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? == 3,
            "任务库版本不支持"
        );
        Ok(Self { db })
    }
    pub fn scan_mode(&self, partition: &str) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM scan_modes WHERE partition_id=?1)",
            [partition],
            |r| r.get(0),
        )?)
    }
    pub fn set_scan_mode(&self, partition: &str, enabled: bool) -> Result<()> {
        if enabled {
            self.db
                .execute("INSERT OR IGNORE INTO scan_modes VALUES(?1)", [partition])?;
        } else {
            self.db
                .execute("DELETE FROM scan_modes WHERE partition_id=?1", [partition])?;
        }
        Ok(())
    }
    pub fn enabled(&self) -> Result<bool> {
        Ok(self
            .db
            .query_row("SELECT enabled FROM policy WHERE id=1", [], |r| r.get(0))?)
    }
    pub fn set_enabled(&self, enabled: bool) -> Result<()> {
        self.db.execute(
            "UPDATE policy SET enabled=?1,revision=revision+1 WHERE id=1",
            [enabled],
        )?;
        Ok(())
    }
    pub fn revision(&self) -> Result<u64> {
        Ok(self
            .db
            .query_row("SELECT revision FROM policy WHERE id=1", [], |r| {
                r.get::<_, i64>(0)
            })?
            .try_into()?)
    }
    pub fn paused(&self, partition: &str) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM paused WHERE partition_id=?1)",
            [partition],
            |r| r.get(0),
        )?)
    }
    pub fn paused_partitions(&self) -> Result<Vec<String>> {
        Ok(self
            .db
            .prepare("SELECT partition_id FROM paused ORDER BY partition_id")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn set_paused(&self, partition: &str, paused: bool) -> Result<()> {
        if paused {
            self.db
                .execute("INSERT OR IGNORE INTO paused VALUES(?1)", [partition])?;
        } else {
            self.db
                .execute("DELETE FROM paused WHERE partition_id=?1", [partition])?;
        }
        Ok(())
    }
    pub fn rebuild_confirmation_pending(&self, partition: &str) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE partition_id=?1 AND kind='rebuild' AND state='needs_confirmation')",
            [partition],
            |r| r.get(0),
        )?)
    }
    pub fn enqueue(
        &mut self,
        partition: &str,
        kind: &str,
        mode: IndexMode,
        automatic: bool,
    ) -> Result<Task> {
        let tx = self.db.transaction()?;
        let existing: Option<String> = tx.query_row("SELECT id FROM tasks WHERE partition_id=?1 AND state IN ('queued','running','waiting')", [partition], |r| r.get(0)).optional()?;
        let id = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        tx.execute("INSERT OR IGNORE INTO tasks(id,partition_id,kind,mode,automatic,state,created,updated) VALUES(?1,?2,?3,?4,?5,'queued',?6,?6)", params![id,partition,kind,serde_json::to_string(&mode)?,automatic,now() as i64])?;
        tx.commit()?;
        self.get(&id)
    }
    pub fn list(&self) -> Result<Vec<Task>> {
        let mut stmt = self.db.prepare("SELECT id,partition_id,kind,mode,automatic,state,created,updated,error FROM tasks ORDER BY seq DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, bool>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })?;
        rows.map(|row| {
            let (id, partition_id, kind, mode, automatic, state, created, updated, error) = row?;
            let phase = (kind == "rebuild" && state == "needs_confirmation")
                .then(|| "needs_rebuild".into());
            Ok(Task {
                id,
                partition_id,
                kind,
                mode: serde_json::from_str(&mode)?,
                automatic,
                state,
                created: created.try_into()?,
                updated: updated.try_into()?,
                error,
                phase,
            })
        })
        .collect()
    }
    pub fn get(&self, id: &str) -> Result<Task> {
        self.list()?
            .into_iter()
            .find(|r| r.id == id)
            .context("任务不存在或已过期")
    }
    pub fn set_state(&mut self, id: &str, state: &str, error: Option<&str>) -> Result<()> {
        self.update_state(id, state, error, None)
    }
    pub fn require_confirmation(&mut self, id: &str, kind: &str, error: &str) -> Result<()> {
        self.update_state(id, "needs_confirmation", Some(error), Some(kind))
    }
    fn update_state(
        &mut self,
        id: &str,
        state: &str,
        error: Option<&str>,
        kind: Option<&str>,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        ensure!(
            tx.execute(
                "UPDATE tasks SET state=?2,updated=?3,error=?4,mode=CASE WHEN ?2='running' AND mode=?5 THEN ?6 ELSE mode END,kind=COALESCE(?7,kind) WHERE id=?1",
                params![id, state, now() as i64, error, serde_json::to_string(&IndexMode::CreateJournal)?, serde_json::to_string(&IndexMode::Journal)?, kind]
            )? == 1,
            "任务不存在"
        );
        if state == "completed" {
            tx.execute(
                "UPDATE tasks SET state='superseded',updated=?2
                 WHERE partition_id=(SELECT partition_id FROM tasks WHERE id=?1)
                   AND state IN ('failed','interrupted','needs_confirmation')",
                params![id, now() as i64],
            )?;
        }
        if !matches!(state, "queued" | "running" | "waiting") {
            Self::prune(&tx)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn cancel(&mut self, id: &str) -> Result<()> {
        let task = self.get(id)?;
        ensure!(
            matches!(task.state.as_str(), "queued" | "running" | "waiting"),
            "任务已结束"
        );
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO paused VALUES(?1)",
            [&task.partition_id],
        )?;
        tx.execute(
            "UPDATE tasks SET state='cancelled',updated=?2 WHERE id=?1",
            params![id, now() as i64],
        )?;
        Self::prune(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn recover(&mut self) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("UPDATE tasks SET state='interrupted',error='服务中断，将从已提交边界恢复',updated=?1 WHERE state='running'", [now() as i64])?;
        Self::prune(&tx)?;
        tx.commit()?;
        Ok(())
    }

    fn prune(tx: &Transaction<'_>) -> Result<()> {
        tx.execute_batch(
            "DELETE FROM tasks WHERE automatic=1 AND kind IN ('sync','scan') AND state='completed';
             UPDATE tasks SET state='superseded'
               WHERE state IN ('failed','interrupted','needs_confirmation')
                 AND seq NOT IN (
                   SELECT max(seq) FROM tasks
                   WHERE state IN ('failed','interrupted','needs_confirmation')
                   GROUP BY partition_id
                 );
             DELETE FROM tasks WHERE seq IN (
               SELECT seq FROM tasks WHERE state IN ('completed','cancelled','superseded')
               ORDER BY updated DESC,seq DESC LIMIT -1 OFFSET 30
             );",
        )?;
        Ok(())
    }
}
