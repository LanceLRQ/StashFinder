use anyhow::{Result, ensure};

/// Basic literal search; advanced Everything expressions are a separate milestone.
pub struct Query {
    terms: Vec<String>,
    names_only: bool,
}

impl Query {
    pub fn parse(text: &str, names_only: bool) -> Result<Self> {
        ensure!(
            text.len() <= 4096 && !text.contains('\0'),
            "搜索文字过长或包含无效字符"
        );
        let mut terms = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut had_quote = false;
        for character in text.chars() {
            if character == '"' {
                quoted = !quoted;
                had_quote = true;
            } else if character.is_whitespace() && !quoted {
                if !current.is_empty() {
                    terms.push(std::mem::take(&mut current));
                } else {
                    ensure!(!had_quote, "引号中的搜索文字不能为空");
                }
                had_quote = false;
            } else {
                current.push(character);
            }
        }
        ensure!(!quoted, "搜索引号未闭合");
        if !current.is_empty() {
            terms.push(current);
        } else {
            ensure!(!had_quote, "引号中的搜索文字不能为空");
        }
        ensure!(terms.len() <= 32, "搜索关键词过多");
        Ok(Self {
            terms: terms
                .into_iter()
                .map(|s| s.replace('/', "\\").to_lowercase())
                .collect(),
            names_only,
        })
    }

    pub fn matches(&self, name: &str, relative_path: &str, mounts: &[String]) -> bool {
        let target = if self.names_only { name } else { relative_path };
        let folded = target.replace('/', "\\").to_lowercase();
        self.terms.iter().all(|term| {
            let bytes = term.as_bytes();
            let explicit_drive = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && bytes[2] == b'\\';
            if explicit_drive {
                !self.names_only
                    && mounts.iter().any(|mount| {
                        let prefix = mount.replace('/', "\\").to_lowercase();
                        let path = format!(
                            "{}\\{}",
                            prefix.trim_end_matches('\\'),
                            folded.trim_start_matches('\\')
                        );
                        path.starts_with(term)
                    })
            } else {
                folded.contains(term)
            }
        })
    }
}
