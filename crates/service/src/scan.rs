use anyhow::{Result, ensure};
use stashfinder_core::assets::{Asset, Identity, IdentityMatch};
use stashfinder_store::volume::{Entry, Object, Volume};
use stashfinder_windows::{
    scan::{ScanEntry, Scanner},
    volumes,
};
use std::{
    collections::VecDeque,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub trait Source {
    fn verify(&self) -> Result<()>;
    fn changed(&mut self) -> Result<bool>;
    fn enumerate(
        &self,
        relative: &[u16],
        cancelled: &AtomicBool,
        consume: &mut dyn FnMut(&[ScanEntry]) -> Result<()>,
    ) -> Result<()>;
}
pub struct Native {
    scanner: Scanner,
    identity: Identity,
    candidate: String,
}
impl Native {
    pub fn open(asset: &Asset) -> Result<Box<dyn Source>> {
        let found = volumes::discover()?;
        let matching = found
            .iter()
            .filter(|o| {
                asset.registration.observation.identity.compare(&o.identity) == IdentityMatch::Same
            })
            .collect::<Vec<_>>();
        ensure!(
            matching.len() == 1 && matching[0].eligible(),
            "扫描分区已断开或身份无法唯一确认"
        );
        Ok(Box::new(Self {
            scanner: Scanner::open(Path::new(&matching[0].candidate_id))?,
            identity: matching[0].identity.clone(),
            candidate: matching[0].candidate_id.clone(),
        }))
    }
}
impl Source for Native {
    fn verify(&self) -> Result<()> {
        let found = volumes::discover()?;
        let matching = found
            .iter()
            .filter(|o| self.identity.compare(&o.identity) == IdentityMatch::Same)
            .collect::<Vec<_>>();
        ensure!(
            matching.len() == 1
                && matching[0].eligible()
                && matching[0].candidate_id == self.candidate,
            "扫描会话已断开或身份变化"
        );
        Ok(())
    }
    fn changed(&mut self) -> Result<bool> {
        self.scanner.changed()
    }
    fn enumerate(
        &self,
        relative: &[u16],
        cancelled: &AtomicBool,
        consume: &mut dyn FnMut(&[ScanEntry]) -> Result<()>,
    ) -> Result<()> {
        self.scanner.enumerate(relative, cancelled, consume)
    }
}

pub fn build(source: &mut dyn Source, db: &mut Volume, cancelled: &AtomicBool) -> Result<()> {
    for attempt in 0..3 {
        ensure!(!cancelled.load(Ordering::Acquire), "扫描已取消");
        source.verify()?;
        // Clear already observed notifications before taking the new snapshot.
        source.changed()?;
        let generation = db.begin_generation()?;
        let result = (|| -> Result<bool> {
            let root = vec![0u8];
            db.write_batch(
                generation,
                &[Object {
                    id: root.clone(),
                    source: "path".into(),
                    kind: "folder".into(),
                    size: None,
                    created: None,
                    modified: None,
                    attributes: 16,
                }],
                &[Entry {
                    id: root.clone(),
                    object: root,
                    parent: None,
                    name: vec![92],
                }],
                &[],
                None,
            )?;
            let mut queue = VecDeque::from([Vec::<u16>::new()]);
            while let Some(relative) = queue.pop_front() {
                ensure!(!cancelled.load(Ordering::Acquire), "扫描已取消");
                let parent = if relative.is_empty() {
                    vec![0]
                } else {
                    relative.iter().flat_map(|c| c.to_le_bytes()).collect()
                };
                source.enumerate(&relative, cancelled, &mut |batch| {
                    let mut objects = Vec::with_capacity(batch.len());
                    let mut entries = Vec::with_capacity(batch.len());
                    for row in batch {
                        let mut path = relative.clone();
                        if !path.is_empty() {
                            path.push(92)
                        }
                        path.extend_from_slice(&row.name);
                        let id: Vec<u8> = path.iter().flat_map(|c| c.to_le_bytes()).collect();
                        let directory = row.attributes & 16 != 0;
                        if directory && row.attributes & 1024 == 0 {
                            queue.push_back(path)
                        }
                        objects.push(Object {
                            id: id.clone(),
                            source: "path".into(),
                            kind: if directory { "folder" } else { "file" }.into(),
                            size: row.size,
                            created: row.created.map(u64::to_le_bytes),
                            modified: row.modified.map(u64::to_le_bytes),
                            attributes: row.attributes,
                        });
                        entries.push(Entry {
                            id: id.clone(),
                            object: id,
                            parent: Some(parent.clone()),
                            name: row.name.clone(),
                        });
                    }
                    db.write_batch(generation, &objects, &entries, &[], None)
                })?;
            }
            source.verify()?;
            ensure!(!cancelled.load(Ordering::Acquire), "扫描已取消");
            if source.changed()? {
                return Ok(false);
            }
            db.complete(generation)?;
            db.publish_snapshot(generation)?;
            Ok(true)
        })();
        match result {
            Ok(true) => return Ok(()),
            Ok(false) => db.fail_generation(generation, "扫描期间文件变化，等待完整校验")?,
            Err(e) => {
                db.fail_generation(generation, &format!("{e:#}"))?;
                return Err(e);
            }
        }
        ensure!(attempt < 2, "文件持续变化，旧索引保留，稍后重试完整校验");
    }
    unreachable!()
}
