use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Page {
    Overview,
    Files,
    Tasks,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    #[serde(default = "default_categories")]
    pub categories: Vec<Category>,
    pub version: u32,
    pub collapsed: bool,
    pub start_page: Page,
    pub include_path: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            collapsed: false,
            start_page: Page::Overview,
            include_path: true,
            categories: default_categories(),
        }
    }
}

pub fn read(path: &Path) -> Result<Preferences> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Preferences::default()),
        Err(e) => return Err(e).with_context(|| format!("无法读取设置：{}", path.display())),
    };
    let mut preferences: Preferences = serde_json::from_slice(&bytes)
        .with_context(|| format!("设置格式损坏，请备份后移走该文件再重启：{}", path.display()))?;
    ensure!(
        preferences.version == 1,
        "设置版本不受支持，请使用匹配的应用版本"
    );
    normalize(&mut preferences)?;
    Ok(preferences)
}

pub fn write(path: &Path, preferences: &Preferences) -> Result<()> {
    let mut preferences = preferences.clone();
    normalize(&mut preferences)?;
    ensure!(preferences.version == 1, "设置版本不受支持");
    read(path)?;
    let parent = path.parent().context("设置路径缺少父目录")?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec_pretty(&preferences)?)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("未能替换设置文件，原设置保留：{}", path.display()))?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub name: String,
    pub extensions: Vec<String>,
}
fn default_categories() -> Vec<Category> {
    [
        ("视频", "mp4,mkv,avi,mov,wmv,webm,ts,m2ts"),
        ("图片", "jpg,jpeg,png,gif,webp,bmp,tif,tiff,heic,svg"),
        ("音频", "mp3,flac,wav,aac,ogg,m4a,ape"),
        ("文档", "pdf,doc,docx,xls,xlsx,ppt,pptx,txt,md,epub"),
        ("压缩包", "zip,rar,7z,tar,gz,bz2,xz"),
        ("程序", "exe,msi,dll,bat,cmd,ps1"),
    ]
    .into_iter()
    .map(|(name, ext)| Category {
        name: name.into(),
        extensions: ext.split(',').map(String::from).collect(),
    })
    .collect()
}
fn normalize(p: &mut Preferences) -> Result<()> {
    ensure!(p.categories.len() <= 32, "文件分类最多 32 个");
    let mut names = std::collections::HashSet::new();
    for c in &mut p.categories {
        c.name = c.name.trim().into();
        ensure!(
            !c.name.is_empty()
                && c.name.len() <= 256
                && !c.name.chars().any(char::is_control)
                && names.insert(c.name.to_lowercase()),
            "分类名称无效或重复"
        );
        ensure!(
            !c.extensions.is_empty() && c.extensions.len() <= 256,
            "分类扩展名数量无效"
        );
        c.extensions = stashfinder_core::search::normalize_extensions(&c.extensions)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn categories_normalize_and_reject_invalid_or_duplicate_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let mut prefs = Preferences {
            categories: vec![Category {
                name: "Custom".into(),
                extensions: vec![".JPG".into(), "jpg".into(), "PNG".into()],
            }],
            ..Default::default()
        };
        write(&path, &prefs).unwrap();
        let saved = read(&path).unwrap();
        assert_eq!(saved.categories[0].extensions, ["jpg", "png"]);
        prefs.categories[0].extensions = vec!["*.jpg".into()];
        assert!(write(&path, &prefs).is_err());
        assert_eq!(read(&path).unwrap(), saved);
        prefs.categories = vec![
            Category {
                name: "Custom".into(),
                extensions: vec!["txt".into()]
            };
            2
        ];
        assert!(write(&path, &prefs).is_err());
    }

    #[test]
    fn preferences_default_roundtrip_and_replace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let defaults = read(&path).unwrap();
        assert_eq!(defaults, Preferences::default());
        assert!(!path.exists());
        let first = Preferences {
            collapsed: true,
            start_page: Page::Files,
            include_path: false,
            ..defaults
        };
        write(&path, &first).unwrap();
        assert_eq!(read(&path).unwrap(), first);
        let second = Preferences {
            start_page: Page::Settings,
            ..first
        };
        write(&path, &second).unwrap();
        assert_eq!(read(&path).unwrap(), second);
    }

    #[test]
    fn preferences_corruption_and_future_version_are_not_silent_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        for content in [
            "broken",
            r#"{"version":2,"collapsed":false,"start_page":"overview","include_path":true}"#,
            r#"{"version":1,"collapsed":false,"start_page":"invalid","include_path":true}"#,
        ] {
            std::fs::write(&path, content).unwrap();
            assert!(read(&path).is_err());
            assert!(write(&path, &Preferences::default()).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        }
        assert!(
            write(
                &path,
                &Preferences {
                    version: 2,
                    ..Preferences::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn preferences_failed_replace_preserves_existing_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        write(&path, &Preferences::default()).unwrap();
        let old = std::fs::read(&path).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        assert!(
            write(
                &path,
                &Preferences {
                    collapsed: true,
                    ..Preferences::default()
                }
            )
            .is_err()
        );
        drop(lock);
        assert_eq!(std::fs::read(&path).unwrap(), old);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
