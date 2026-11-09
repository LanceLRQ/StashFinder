use crate::volume::{Entry, Object, Volume};
use anyhow::{Result, ensure};
use rusqlite::{TransactionBehavior, params};

#[derive(Debug)]
pub struct InitialPathsIncomplete;

impl std::fmt::Display for InitialPathsIncomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("初始路径缺少对应的父目录")
    }
}

impl std::error::Error for InitialPathsIncomplete {}

impl Volume {
    pub fn begin_initial_paths(&mut self, generation: i64) -> Result<()> {
        self.db
            .execute_batch("PRAGMA temp_store=FILE; PRAGMA temp.cache_size=-8192;")?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(tx.execute("UPDATE generations SET phase='initial_paths' WHERE id=?1 AND phase='building' AND complete=0
            AND NOT EXISTS(SELECT 1 FROM index_state WHERE active_generation=?1)
            AND NOT EXISTS(SELECT 1 FROM objects WHERE generation=?1)", [generation])? == 1, "初始路径需要空的未发布代次");
        tx.execute_batch("CREATE TEMP TABLE initial_paths (id BLOB PRIMARY KEY, object BLOB NOT NULL, parent BLOB, name BLOB NOT NULL) WITHOUT ROWID;
            CREATE INDEX temp.initial_paths_parent ON initial_paths(parent);
            CREATE UNIQUE INDEX temp.initial_paths_name ON initial_paths(parent,name);")?;
        tx.commit()?;
        Ok(())
    }

    pub fn write_initial_batch(
        &mut self,
        generation: i64,
        objects: &[Object],
        entries: &[Entry],
    ) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(
            tx.execute(
                "UPDATE generations SET revision=revision+1 WHERE id=?1 AND phase='initial_paths'",
                [generation]
            )? == 1,
            "初始路径代次未打开"
        );
        {
            let mut object_insert = tx.prepare_cached("INSERT INTO objects(generation,id,source,kind,size,created,modified,attributes) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
            let mut objects: Vec<_> = objects.iter().collect();
            objects.sort_unstable_by(|a, b| a.id.cmp(&b.id));
            for object in objects {
                ensure!(
                    !object.id.is_empty()
                        && object.source == "ntfs64"
                        && object.identity.is_none()
                        && object.created.is_some()
                        && object.modified.is_some()
                        && (object.kind == "file") == object.size.is_some(),
                    "顺序索引对象的元数据不完整"
                );
                object_insert.execute(params![
                    generation,
                    object.id,
                    object.source,
                    object.kind,
                    object.size,
                    object.created.as_ref().map(|v| v.as_slice()),
                    object.modified.as_ref().map(|v| v.as_slice()),
                    object.attributes
                ])?;
            }
            let mut path_insert = tx.prepare_cached(
                "INSERT INTO temp.initial_paths(id,object,parent,name) VALUES(?1,?2,?3,?4)",
            )?;
            for entry in entries {
                ensure!(
                    !entry.id.is_empty() && !entry.name.is_empty(),
                    "初始路径标识或名称为空"
                );
                let name: Vec<u8> = entry
                    .name
                    .iter()
                    .flat_map(|unit| unit.to_le_bytes())
                    .collect();
                path_insert.execute(params![entry.id, entry.object, entry.parent, name])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn finish_initial_paths(&mut self, generation: i64, root: &[u8]) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(tx.execute("UPDATE generations SET phase='building',revision=revision+1 WHERE id=?1 AND phase='initial_paths'", [generation])? == 1, "初始路径代次未打开");
        let valid_root: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM temp.initial_paths p JOIN objects o ON o.generation=?1 AND o.id=p.object
            WHERE p.id=?2 AND p.parent IS NULL AND o.kind='folder') AND NOT EXISTS(SELECT 1 FROM temp.initial_paths WHERE parent IS NULL AND id!=?2)", params![generation,root], |r|r.get(0))?;
        ensure!(valid_root, "初始路径根目录不唯一或无效");
        let invalid_object: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM temp.initial_paths p LEFT JOIN objects o ON o.generation=?1 AND o.id=p.object WHERE o.id IS NULL)", [generation], |r|r.get(0))?;
        ensure!(!invalid_object, "初始路径缺少文件对象");
        let missing_parent: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM temp.initial_paths p LEFT JOIN temp.initial_paths parent ON parent.id=p.parent WHERE p.parent IS NOT NULL AND parent.id IS NULL)", [], |r|r.get(0))?;
        if missing_parent {
            return Err(InitialPathsIncomplete.into());
        }
        let non_directory_parent: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM temp.initial_paths p JOIN temp.initial_paths parent ON parent.id=p.parent JOIN objects o ON o.generation=?1 AND o.id=parent.object WHERE o.kind!='folder')", [generation], |r|r.get(0))?;
        ensure!(!non_directory_parent, "初始路径父节点不是目录");

        // Every path has one parent. A disconnected cycle cannot be reached from the root.
        tx.execute_batch("CREATE TEMP TABLE initial_reachable(id BLOB PRIMARY KEY, visible INTEGER NOT NULL) WITHOUT ROWID;")?;
        tx.execute("WITH RECURSIVE tree(id,attributes,visible) AS (
            SELECT p.id,o.attributes,1 FROM temp.initial_paths p JOIN objects o ON o.generation=?1 AND o.id=p.object WHERE p.id=?2
            UNION ALL
            SELECT child.id,o.attributes,tree.visible AND (tree.attributes & 1024)=0 FROM tree
            JOIN temp.initial_paths child ON child.parent=tree.id JOIN objects o ON o.generation=?1 AND o.id=child.object
        ) INSERT INTO temp.initial_reachable SELECT id,visible FROM tree", params![generation,root])?;
        let complete: bool = tx.query_row("SELECT (SELECT count(*) FROM temp.initial_reachable)=(SELECT count(*) FROM temp.initial_paths)", [], |r|r.get(0))?;
        ensure!(complete, "初始路径父链存在循环");
        tx.execute("INSERT INTO paths(generation,id,object,parent,name) SELECT ?1,p.id,p.object,p.parent,p.name
            FROM temp.initial_paths p JOIN temp.initial_reachable r ON r.id=p.id WHERE r.visible ORDER BY p.id", [generation])?;
        tx.execute("DELETE FROM objects WHERE generation=?1 AND id NOT IN (SELECT object FROM paths WHERE generation=?1)", [generation])?;
        tx.execute_batch("DROP TABLE temp.initial_reachable; DROP TABLE temp.initial_paths;")?;
        tx.commit()?;
        Ok(())
    }

    pub fn discard_initial(&mut self, generation: i64, reason: &str) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(tx.execute("UPDATE generations SET phase='failed',complete=0,error=?2,journal=NULL,next_usn=NULL WHERE id=?1
            AND phase IN ('initial_paths','building') AND NOT EXISTS(SELECT 1 FROM index_state WHERE active_generation=?1)", params![generation,reason])? == 1, "只能清理未发布的构建代次");
        tx.execute("DELETE FROM paths WHERE generation=?1", [generation])?;
        tx.execute(
            "DELETE FROM pending_metadata WHERE generation=?1",
            [generation],
        )?;
        tx.execute("DELETE FROM objects WHERE generation=?1", [generation])?;
        tx.execute_batch("DROP TABLE IF EXISTS temp.initial_paths;")?;
        tx.commit()?;
        Ok(())
    }
}
