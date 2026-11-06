use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::PathBuf,
    time::{Duration, SystemTime},
};

pub const DEFAULT_LIMIT: u64 = 2_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key([u8; 32]);
impl Key {
    pub fn new(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
    fn filename(self) -> String {
        let hex: String = self.0.iter().map(|b| format!("{b:02x}")).collect();
        format!("{hex}.png")
    }
    fn from_filename(name: &str) -> Option<Self> {
        let hex = name.strip_suffix(".png")?;
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        let mut bytes = [0; 32];
        for (index, value) in bytes.iter_mut().enumerate() {
            *value = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
        }
        Some(Self(bytes))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u32,
    limit_bytes: u64,
}

#[derive(Clone, Serialize)]
pub struct Stats {
    pub used_bytes: u64,
    pub limit_bytes: u64,
    pub files: usize,
}
#[derive(Clone, Serialize)]
pub struct Progress {
    pub processed: usize,
    pub total: usize,
    pub used_bytes: u64,
    pub limit_bytes: u64,
}

struct Entry {
    bytes: u64,
    touched: SystemTime,
}
pub struct Cache {
    root: PathBuf,
    config: PathBuf,
    limit: u64,
    used: u64,
    entries: HashMap<Key, Entry>,
    clock: SystemTime,
}
impl Cache {
    pub fn open(root: PathBuf, config: PathBuf) -> Result<Self> {
        let limit = match fs::read(&config) {
            Ok(bytes) => {
                let stored: Config =
                    serde_json::from_slice(&bytes).context("图片缓存设置损坏，请检查缓存配置")?;
                ensure!(stored.version == 1, "图片缓存设置版本不受支持");
                validate_limit(stored.limit_bytes)?;
                stored.limit_bytes
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => DEFAULT_LIMIT,
            Err(error) => return Err(error).context("无法读取图片缓存设置"),
        };
        fs::create_dir_all(&root).context("无法创建图片缓存目录")?;
        let mut entries = HashMap::new();
        let mut used = 0;
        for item in fs::read_dir(&root)? {
            let item = item?;
            if !item.file_type()?.is_file() {
                continue;
            }
            let Some(key) = Key::from_filename(&item.file_name().to_string_lossy()) else {
                continue;
            };
            let metadata = item.metadata()?;
            used += metadata.len();
            entries.insert(
                key,
                Entry {
                    bytes: metadata.len(),
                    touched: metadata.modified()?,
                },
            );
        }
        let mut cache = Self {
            root,
            config,
            limit,
            used,
            entries,
            clock: SystemTime::now(),
        };
        cache.trim(limit, |_| {})?;
        Ok(cache)
    }
    pub fn stats(&self) -> Stats {
        Stats {
            used_bytes: self.used,
            limit_bytes: self.limit,
            files: self.entries.len(),
        }
    }
    pub fn get(&mut self, key: Key) -> Result<Option<Vec<u8>>> {
        if !self.entries.contains_key(&key) {
            return Ok(None);
        }
        let path = self.root.join(key.filename());
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.used -= self.entries.remove(&key).unwrap().bytes;
                return Ok(None);
            }
            Err(error) => return Err(error).context("无法读取图片缓存"),
        };
        self.clock = SystemTime::now().max(self.clock + Duration::from_micros(1));
        fs::OpenOptions::new()
            .write(true)
            .open(&path)?
            .set_times(fs::FileTimes::new().set_modified(self.clock))?;
        self.entries.get_mut(&key).unwrap().touched = self.clock;
        Ok(Some(bytes))
    }
    pub fn put(&mut self, key: Key, bytes: &[u8]) -> Result<bool> {
        if bytes.len() as u64 > self.limit {
            self.remove(key)?;
            return Ok(false);
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(self.root.join(key.filename()))
            .map_err(|e| e.error)
            .context("无法保存图片缓存")?;
        if let Some(old) = self.entries.remove(&key) {
            self.used -= old.bytes;
        }
        self.clock = SystemTime::now().max(self.clock + Duration::from_micros(1));
        self.used += bytes.len() as u64;
        self.entries.insert(
            key,
            Entry {
                bytes: bytes.len() as u64,
                touched: self.clock,
            },
        );
        self.trim(self.limit, |_| {})?;
        Ok(true)
    }
    pub fn set_limit(&mut self, limit: u64, progress: impl FnMut(Progress)) -> Result<()> {
        validate_limit(limit)?;
        let parent = self.config.parent().context("缓存配置路径无父目录")?;
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&serde_json::to_vec(&Config {
            version: 1,
            limit_bytes: limit,
        })?)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.config)
            .map_err(|e| e.error)
            .context("无法保存图片缓存设置")?;
        self.limit = limit;
        self.trim(limit, progress)
    }
    pub fn clear(&mut self, progress: impl FnMut(Progress)) -> Result<()> {
        self.trim(0, progress)
    }
    fn remove(&mut self, key: Key) -> Result<()> {
        if let Some(entry) = self.entries.get(&key) {
            match fs::remove_file(self.root.join(key.filename())) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("无法整理图片缓存"),
            }
            self.used -= entry.bytes;
            self.entries.remove(&key);
        }
        Ok(())
    }
    fn trim(&mut self, ceiling: u64, mut progress: impl FnMut(Progress)) -> Result<()> {
        if self.used <= ceiling {
            progress(Progress {
                processed: 0,
                total: 0,
                used_bytes: self.used,
                limit_bytes: self.limit,
            });
            return Ok(());
        }
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .map(|(key, entry)| (*key, entry.touched))
            .collect();
        entries.sort_unstable_by_key(|(_, touched)| *touched);
        let mut remaining = self.used;
        let victims: Vec<_> = entries
            .into_iter()
            .take_while(|(key, _)| {
                if remaining <= ceiling {
                    return false;
                }
                remaining -= self.entries[key].bytes;
                true
            })
            .map(|(key, _)| key)
            .collect();
        let total = victims.len();
        progress(Progress {
            processed: 0,
            total,
            used_bytes: self.used,
            limit_bytes: self.limit,
        });
        for (index, key) in victims.into_iter().enumerate() {
            self.remove(key)?;
            progress(Progress {
                processed: index + 1,
                total,
                used_bytes: self.used,
                limit_bytes: self.limit,
            });
        }
        Ok(())
    }
}
fn validate_limit(limit: u64) -> Result<()> {
    ensure!(
        limit > 0 && limit <= 9_007_199_254_740_991,
        "缓存上限必须是可精确表示的正整数字节数"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintenance_progress_counts_the_actual_batch_and_remaining_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let mut cache =
            Cache::open(temp.path().join("cache"), temp.path().join("config.json")).unwrap();
        cache.put(Key::new(b"first"), b"123").unwrap();
        cache.put(Key::new(b"second"), b"1234").unwrap();
        cache.put(Key::new(b"third"), b"12345").unwrap();
        let mut events = Vec::new();
        cache
            .set_limit(7, |progress| events.push(progress))
            .unwrap();
        assert_eq!(events.first().unwrap().processed, 0);
        assert_eq!(events.last().unwrap().processed, 2);
        assert_eq!(events.last().unwrap().total, 2);
        assert_eq!(events.last().unwrap().used_bytes, 5);
        events.clear();
        cache.clear(|progress| events.push(progress)).unwrap();
        assert_eq!(events.last().unwrap().processed, 1);
        assert_eq!(events.last().unwrap().total, 1);
        assert_eq!(events.last().unwrap().used_bytes, 0);
    }

    #[test]
    fn byte_limit_lru_restart_and_clear_preserve_configuration_and_other_data() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let config = temp.path().join("config/cache.json");
        let index = temp.path().join("index.sqlite");
        std::fs::write(&index, b"index must survive").unwrap();
        let mut cache = Cache::open(root.clone(), config.clone()).unwrap();
        assert_eq!(cache.stats().limit_bytes, 2_000_000_000);
        let a = Key::new(b"a");
        let b = Key::new(b"b");
        cache.put(a, b"first").unwrap();
        cache.put(b, b"second").unwrap();
        assert_eq!(cache.get(a).unwrap().unwrap(), b"first");
        cache.set_limit(6, |_| {}).unwrap();
        assert!(cache.get(b).unwrap().is_none());
        assert_eq!(cache.stats().used_bytes, 5);
        assert!(!cache.put(b, b"too large").unwrap());
        assert_eq!(cache.stats().files, 1);
        drop(cache);
        let mut cache = Cache::open(root, config.clone()).unwrap();
        assert_eq!(cache.stats().limit_bytes, 6);
        assert_eq!(cache.get(a).unwrap().unwrap(), b"first");
        cache.clear(|_| {}).unwrap();
        assert_eq!(cache.stats().used_bytes, 0);
        assert_eq!(cache.stats().files, 0);
        assert_eq!(std::fs::read(index).unwrap(), b"index must survive");
        assert!(config.exists());
        assert_eq!(
            Cache::open(cache.root.clone(), config)
                .unwrap()
                .stats()
                .limit_bytes,
            6
        );
    }

    #[test]
    fn invalid_configuration_is_reported_and_unknown_files_are_not_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let config = temp.path().join("config.json");
        std::fs::write(&config, b"corrupt").unwrap();
        assert!(Cache::open(root.clone(), config.clone()).is_err());
        assert_eq!(std::fs::read(&config).unwrap(), b"corrupt");
        std::fs::remove_file(&config).unwrap();
        let mut cache = Cache::open(root.clone(), config).unwrap();
        let outside = root.join("unrelated.txt");
        std::fs::write(&outside, b"keep").unwrap();
        cache.put(Key::new(b"known"), b"image").unwrap();
        assert!(cache.set_limit(0, |_| {}).is_err());
        cache.clear(|_| {}).unwrap();
        assert_eq!(std::fs::read(outside).unwrap(), b"keep");
    }

    #[test]
    fn replacement_and_missing_cache_files_keep_byte_accounting_correct() {
        let temp = tempfile::tempdir().unwrap();
        let mut cache =
            Cache::open(temp.path().join("cache"), temp.path().join("config.json")).unwrap();
        let key = Key::new(b"same");
        cache.put(key, b"old").unwrap();
        cache.put(key, b"replacement").unwrap();
        assert_eq!(cache.stats().used_bytes, 11);
        assert_eq!(cache.stats().files, 1);
        assert_eq!(cache.get(key).unwrap().unwrap(), b"replacement");
        std::fs::remove_file(cache.root.join(key.filename())).unwrap();
        assert!(cache.get(key).unwrap().is_none());
        assert_eq!(cache.stats().used_bytes, 0);
    }
}
