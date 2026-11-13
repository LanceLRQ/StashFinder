use crate::index::SearchRow;
use anyhow::{Context, Result, bail, ensure};
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use pcre2::bytes::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    #[default]
    Contains,
    WholeName,
    WholeWord,
    Start,
    End,
    Wildcard,
    Regex,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateField {
    Name,
    Size,
    Modified,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    #[default]
    Name,
    Path,
    Partition,
    Size,
    Created,
    Modified,
    Group,
    Extension,
    Type,
    Attributes,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Range {
    pub min: Option<String>,
    pub max: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileFilter {
    pub extensions: Vec<String>,
    pub include_folders: bool,
    pub kind: Option<String>,
    pub size: Range,
    pub created: Range,
    pub modified: Range,
}

impl FileFilter {
    pub fn compile(&self) -> Result<Query> {
        Query::compile(
            "",
            false,
            &QueryOptions {
                extensions: self.extensions.clone(),
                include_folders: self.include_folders,
                kind: self.kind.clone(),
                size: self.size.clone(),
                created: self.created.clone(),
                modified: self.modified.clone(),
                ..QueryOptions::default()
            },
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub partition_id: String,
    pub entry: Vec<u8>,
    pub recursive: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueryOptions {
    pub include_folders: bool,
    pub case_sensitive: bool,
    pub mode: MatchMode,
    pub extensions: Vec<String>,
    pub kind: Option<String>,
    pub size: Range,
    pub created: Range,
    pub modified: Range,
    pub online_only: bool,
    pub include_history: bool,
    pub scope: Option<Scope>,
    pub duplicate: Vec<DuplicateField>,
    pub sort: SortField,
    pub descending: bool,
}
#[derive(Debug)]
enum Expr {
    All,
    Term(Term),
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}
#[derive(Debug)]
enum Term {
    TypeExtensions(Vec<String>, bool),
    Text(Text),
    Kind(bool),
    Extension(Vec<String>),
    Number(u8, Bounds),
}
#[derive(Debug)]
struct Text {
    value: String,
    target: u8,
    mode: MatchMode,
    regex: Option<Regex>,
    case: bool,
}
#[derive(Clone, Copy, Debug, Default)]
struct Bounds {
    min: Option<u64>,
    max: Option<u64>,
}
#[derive(Debug)]
pub struct Query {
    expression: Expr,
    filters: Vec<Term>,
    duplicate: Vec<DuplicateField>,
    names_only: bool,
}
pub struct Record<'a> {
    pub name: &'a str,
    pub folded_name: &'a str,
    pub parent: &'a str,
    pub folded_parent: &'a str,
    pub kind: &'a str,
    pub size: Option<u64>,
    pub created: Option<u64>,
    pub modified: Option<u64>,
}
impl Query {
    pub fn parse(text: &str, names_only: bool) -> Result<Self> {
        Self::compile(text, names_only, &Default::default())
    }
    pub fn compile(text: &str, names_only: bool, o: &QueryOptions) -> Result<Self> {
        ensure!(
            text.len() <= 4096 && !text.contains('\0'),
            "搜索文字过长或包含无效字符"
        );
        ensure!(
            o.extensions.len() <= 256 && o.duplicate.len() <= 3,
            "搜索条件过多"
        );
        let mut p = Parser {
            tokens: if o.mode == MatchMode::Regex {
                vec![]
            } else {
                tokens(text)?
            },
            pos: 0,
            depth: 0,
            options: o,
            duplicate: o.duplicate.clone(),
        };
        let expression = if o.mode == MatchMode::Regex {
            Expr::Term(Term::Text(Text::new(
                text,
                0,
                MatchMode::Regex,
                o.case_sensitive,
            )?))
        } else if p.tokens.is_empty() {
            Expr::All
        } else {
            let e = p.and()?;
            ensure!(p.pos == p.tokens.len(), "搜索分组或运算符不匹配");
            e
        };
        let mut filters = Vec::new();
        if !o.extensions.is_empty() {
            filters.push(Term::TypeExtensions(
                normalize_extensions(&o.extensions)?,
                o.include_folders,
            ));
        }
        if let Some(kind) = &o.kind {
            ensure!(matches!(kind.as_str(), "file" | "folder"), "无效的文件类型");
            filters.push(Term::Kind(kind == "folder"));
        }
        for (field, r) in [(0, &o.size), (1, &o.created), (2, &o.modified)] {
            let b = Bounds {
                min: r
                    .min
                    .as_deref()
                    .map(str::parse)
                    .transpose()
                    .context("筛选下限无效")?,
                max: r
                    .max
                    .as_deref()
                    .map(str::parse)
                    .transpose()
                    .context("筛选上限无效")?,
            };
            ensure!(
                b.min.zip(b.max).is_none_or(|(a, b)| a <= b),
                "筛选下限不能大于上限"
            );
            if b.min.is_some() || b.max.is_some() {
                filters.push(Term::Number(field, b));
            }
        }
        let mut duplicate = Vec::new();
        for field in p.duplicate {
            if !duplicate.contains(&field) {
                duplicate.push(field);
            }
        }
        Ok(Self {
            expression,
            filters,
            duplicate,
            names_only,
        })
    }
    pub fn duplicate(&self) -> &[DuplicateField] {
        &self.duplicate
    }
    pub fn matches_row(&self, r: &SearchRow, m: &[String]) -> Result<bool> {
        let path = r.path.replace('/', "\\");
        let parent = path.rsplit_once('\\').map_or("", |(p, _)| p);
        let name = r.name.to_lowercase();
        let folded_parent = parent.to_lowercase();
        self.matches_record(
            &Record {
                name: &r.name,
                folded_name: &name,
                parent,
                folded_parent: &folded_parent,
                kind: &r.kind,
                size: r.size.as_deref().map(str::parse).transpose()?,
                created: r.created.as_deref().map(str::parse).transpose()?,
                modified: r.modified.as_deref().map(str::parse).transpose()?,
            },
            m,
        )
    }
    pub fn matches_record(&self, r: &Record<'_>, m: &[String]) -> Result<bool> {
        for f in &self.filters {
            if !f.matches(r, m, self.names_only)? {
                return Ok(false);
            }
        }
        self.expression.matches(r, m, self.names_only)
    }
    /// Legacy literal benchmark API. Production queries use the fallible row matcher.
    pub fn matches(&self, name: &str, path: &str, mounts: &[String]) -> bool {
        self.matches_row(
            &SearchRow {
                entry: vec![],
                parent: vec![],
                object: vec![],
                name: name.into(),
                path: path.into(),
                kind: "file".into(),
                attributes: 0,
                size: None,
                created: None,
                modified: None,
            },
            mounts,
        )
        .unwrap_or(false)
    }
}
pub fn normalize_extensions(values: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for value in values {
        for ext in value.split([',', ';']) {
            let ext = ext.trim().trim_start_matches('.').to_lowercase();
            ensure!(
                !ext.is_empty()
                    && ext.len() <= 64
                    && ext
                        .chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '+')),
                "扩展名无效，请使用不带路径或通配符的扩展名"
            );
            if !out.contains(&ext) {
                out.push(ext);
            }
        }
    }
    Ok(out)
}
impl Expr {
    fn matches(&self, r: &Record, m: &[String], n: bool) -> Result<bool> {
        match self {
            Self::All => Ok(true),
            Self::Term(t) => t.matches(r, m, n),
            Self::Not(e) => Ok(!e.matches(r, m, n)?),
            Self::And(es) => {
                for e in es {
                    if !e.matches(r, m, n)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or(es) => {
                for e in es {
                    if e.matches(r, m, n)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }
}
impl Term {
    fn matches(&self, r: &Record, m: &[String], n: bool) -> Result<bool> {
        Ok(match self {
            Self::Text(t) => return t.matches(r, m, n),
            Self::Kind(folder) => (r.kind == "folder") == *folder,
            Self::TypeExtensions(_, true) if r.kind == "folder" => true,
            Self::Extension(xs) | Self::TypeExtensions(xs, _) => {
                r.kind == "file"
                    && r.folded_name
                        .rsplit_once('.')
                        .is_some_and(|(_, ext)| xs.iter().any(|x| x == ext))
            }
            Self::Number(field, b) => {
                if *field == 0 && r.kind == "folder" {
                    return Ok(false);
                }
                let v = match field {
                    0 => &r.size,
                    1 => &r.created,
                    _ => &r.modified,
                };
                v.is_some_and(|v| b.min.is_none_or(|x| v >= x) && b.max.is_none_or(|x| v <= x))
            }
        })
    }
}
fn escaped(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if "\\.^$|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
impl Text {
    fn new(value: &str, target: u8, mode: MatchMode, case: bool) -> Result<Self> {
        let wildcard = mode == MatchMode::Wildcard
            || (mode == MatchMode::Contains && value.contains(['*', '?']));
        let value = if mode == MatchMode::Regex {
            value.to_owned()
        } else {
            value.replace('/', "\\")
        };
        let mut target = target;
        let pattern = if wildcard {
            if target == 0 && !value.contains('\\') {
                target = 1;
            }
            let mut p = String::from("\\A");
            for c in value.chars() {
                match c {
                    '*' => p.push_str(".*"),
                    '?' => p.push('.'),
                    _ => p.push_str(&escaped(&c.to_string())),
                }
            }
            p.push_str("\\z");
            Some(p)
        } else if mode == MatchMode::Regex {
            Some(value.clone())
        } else if mode == MatchMode::WholeWord {
            Some(format!("\\b{}\\b", escaped(&value)))
        } else {
            None
        };
        let regex = pattern
            .map(|p| {
                RegexBuilder::new()
                    .utf(true)
                    .ucp(true)
                    .caseless(!case)
                    .build(&format!(
                        "(*LIMIT_MATCH=100000)(*LIMIT_DEPTH=128)(*LIMIT_HEAP=4096){p}"
                    ))
            })
            .transpose()
            .context("正则表达式无效")?;
        Ok(Self {
            value: if case || mode == MatchMode::Regex {
                value
            } else {
                value.to_lowercase()
            },
            target,
            mode,
            regex,
            case,
        })
    }
    fn matches(&self, r: &Record<'_>, mounts: &[String], names_only: bool) -> Result<bool> {
        let b = self.value.as_bytes();
        let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
        let name = if self.case { r.name } else { r.folded_name };
        let parent = if self.case { r.parent } else { r.folded_parent };
        let filename = self.target == 1
            || (self.target == 0
                && (names_only
                    || matches!(
                        self.mode,
                        MatchMode::WholeName | MatchMode::Start | MatchMode::End
                    )));
        if self.regex.is_none() && !drive {
            if self.target == 3 {
                return Ok(parent == self.value);
            }
            if self.target == 4 {
                return Ok(parent == self.value
                    || parent
                        .strip_prefix(&self.value)
                        .is_some_and(|p| p.starts_with('\\')));
            }
            if filename {
                return Ok(match self.mode {
                    MatchMode::WholeName => name == self.value,
                    MatchMode::Start => name.starts_with(&self.value),
                    MatchMode::End => name.ends_with(&self.value),
                    _ => name.contains(&self.value),
                });
            }
            if !self.value.contains('\\') {
                return Ok(name.contains(&self.value) || parent.contains(&self.value));
            }
            let path = if parent.is_empty() {
                name.into()
            } else {
                format!("{parent}\\{name}")
            };
            return Ok(path.contains(&self.value));
        }
        let path = if filename || r.parent.is_empty() && self.target < 3 {
            Cow::Borrowed(r.name)
        } else if self.target == 3 || self.target == 4 {
            Cow::Borrowed(r.parent)
        } else {
            Cow::Owned(format!("{}\\{}", r.parent, r.name))
        };
        if drive {
            if names_only && self.target == 0 {
                return Ok(false);
            }
            for mount in mounts {
                if self.test(
                    &format!(
                        "{}\\{}",
                        mount.trim_end_matches(['\\', '/']),
                        path.trim_start_matches('\\')
                    ),
                    true,
                )? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        self.test(&path, false)
    }
    fn test(&self, s: &str, drive: bool) -> Result<bool> {
        if let Some(re) = &self.regex {
            return re
                .is_match(s.as_bytes())
                .context("正则匹配超过资源限制或执行失败");
        }
        let s = if self.case {
            Cow::Borrowed(s)
        } else {
            Cow::Owned(s.to_lowercase())
        };
        Ok(if self.target == 3 || self.mode == MatchMode::WholeName {
            s == self.value
        } else if self.target == 4 {
            s == self.value
                || s.strip_prefix(&self.value)
                    .is_some_and(|x| x.starts_with('\\'))
        } else if drive || self.mode == MatchMode::Start {
            s.starts_with(&self.value)
        } else if self.mode == MatchMode::End {
            s.ends_with(&self.value)
        } else {
            s.contains(&self.value)
        })
    }
}
#[derive(Debug)]
struct Token {
    value: String,
    literal: bool,
}
fn tokens(text: &str) -> Result<Vec<Token>> {
    let mut out = Vec::new();
    let mut cs = text.chars().peekable();
    while let Some(c) = cs.peek().copied() {
        if c.is_whitespace() {
            cs.next();
            continue;
        }
        if "<>|!".contains(c) {
            out.push(Token {
                value: c.into(),
                literal: false,
            });
            cs.next();
            continue;
        }
        let literal = c == '"';
        let mut v = String::new();
        let mut quoted = false;
        let mut had_quote = false;
        while let Some(c) = cs.peek().copied() {
            if c == '"' {
                quoted = !quoted;
                had_quote = true;
                cs.next();
                continue;
            }
            if !quoted
                && (c.is_whitespace() || "<>|".contains(c))
                && !(v.ends_with(':') && "<>".contains(c))
            {
                break;
            }
            v.push(c);
            cs.next();
        }
        ensure!(!quoted, "搜索引号未闭合");
        ensure!(!v.is_empty() || !had_quote, "引号中的搜索文字不能为空");
        out.push(Token { value: v, literal });
        ensure!(out.len() <= 96, "搜索关键词过多");
    }
    Ok(out)
}
struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
    options: &'a QueryOptions,
    duplicate: Vec<DuplicateField>,
}
impl Parser<'_> {
    fn at(&self, s: &str) -> bool {
        self.tokens
            .get(self.pos)
            .is_some_and(|t| !t.literal && t.value == s)
    }
    fn and(&mut self) -> Result<Expr> {
        let mut es = Vec::new();
        while self.pos < self.tokens.len() && !self.at(">") {
            es.push(self.or()?);
        }
        ensure!(!es.is_empty(), "搜索分组不能为空");
        Ok(Expr::And(es))
    }
    fn or(&mut self) -> Result<Expr> {
        let mut es = vec![self.atom(true)?];
        while self.at("|") {
            self.pos += 1;
            es.push(self.atom(false)?);
        }
        Ok(Expr::Or(es))
    }
    fn atom(&mut self, dupe_allowed: bool) -> Result<Expr> {
        if self.at("!") {
            self.pos += 1;
            self.depth += 1;
            ensure!(self.depth <= 24, "搜索表达式嵌套过深");
            let expression = self.atom(false)?;
            self.depth -= 1;
            return Ok(Expr::Not(Box::new(expression)));
        }
        if self.at("<") {
            self.pos += 1;
            self.depth += 1;
            ensure!(self.depth <= 24, "搜索分组过深");
            let e = self.and()?;
            ensure!(self.at(">"), "搜索分组未闭合");
            self.pos += 1;
            self.depth -= 1;
            return Ok(e);
        }
        let t = self.tokens.get(self.pos).context("搜索运算符缺少条件")?;
        ensure!(
            t.literal || !["|", ">"].contains(&t.value.as_str()),
            "搜索运算符缺少条件"
        );
        self.pos += 1;
        let v = t.value.clone();
        let literal = t.literal;
        let (f, arg) = if !literal {
            v.split_once(':').unwrap_or(("", &v))
        } else {
            ("", v.as_str())
        };
        let function = f.to_ascii_lowercase();
        if ["dupe", "dupesize", "sizedupe"].contains(&function.as_str()) {
            ensure!(
                dupe_allowed && self.depth == 0 && !self.at("|"),
                "查重条件必须放在分组外，不能用于 OR 或排除"
            );
            for p in (if function != "dupe" {
                "size"
            } else if arg.is_empty() {
                "name"
            } else {
                arg
            })
            .split(';')
            {
                self.duplicate.push(match p {
                    "name" => DuplicateField::Name,
                    "size" => DuplicateField::Size,
                    "dm" => DuplicateField::Modified,
                    _ => bail!("不支持的查重属性：{p}"),
                });
            }
            return Ok(Expr::All);
        }
        let term = match function.as_str() {
            "" => Term::Text(Text::new(
                &v,
                0,
                self.options.mode,
                self.options.case_sensitive,
            )?),
            "file" | "folder" => {
                ensure!(arg.is_empty(), "file:/folder: 不接受参数");
                Term::Kind(function == "folder")
            }
            "ext" => Term::Extension(normalize_extensions(&[arg.into()])?),
            "size" | "dc" | "dm" => Term::Number(
                match function.as_str() {
                    "size" => 0,
                    "dc" => 1,
                    _ => 2,
                },
                bounds(arg, function != "size")?,
            ),
            "path" | "parent" | "ancestor" | "regex" | "name" => {
                ensure!(!arg.is_empty(), "搜索函数缺少参数");
                Term::Text(Text::new(
                    arg,
                    match function.as_str() {
                        "path" => 2,
                        "parent" => 3,
                        "ancestor" => 4,
                        "name" => 1,
                        _ => 0,
                    },
                    if function == "regex" {
                        MatchMode::Regex
                    } else {
                        self.options.mode
                    },
                    self.options.case_sensitive,
                )?)
            }
            _ if f.len() == 1 && arg.starts_with(['\\', '/']) => Term::Text(Text::new(
                &v,
                0,
                self.options.mode,
                self.options.case_sensitive,
            )?),
            _ => bail!("不支持的搜索函数：{f}"),
        };
        Ok(Expr::Term(term))
    }
}
fn quantity(s: &str) -> Result<u64> {
    let pos = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let (n, u) = s.split_at(pos);
    let k = match u.to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "kb" => 1000,
        "mb" => 1000000,
        "gb" => 1000000000,
        "tb" => 1000000000000,
        "kib" => 1024,
        "mib" => 1048576,
        "gib" => 1073741824,
        "tib" => 1099511627776,
        _ => bail!("不支持的大小单位"),
    };
    let (a, b) = n.split_once('.').unwrap_or((n, ""));
    ensure!(b.len() <= 6, "大小精度过高");
    let scale = 10u64.pow(b.len() as u32);
    let a = a.parse::<u64>()?.checked_mul(k).context("大小超出范围")?;
    a.checked_add(if b.is_empty() {
        0
    } else {
        b.parse::<u64>()?.checked_mul(k).context("大小超出范围")? / scale
    })
    .context("大小超出范围")
}
fn date(s: &str) -> Result<(u64, u64)> {
    let today = Local::now().date_naive();
    let (a, b) = match s.to_ascii_lowercase().as_str() {
        "today" => (today, today + Duration::days(1)),
        "yesterday" => (today - Duration::days(1), today),
        "thisweek" => {
            let d = today - Duration::days(today.weekday().num_days_from_monday().into());
            (d, d + Duration::days(7))
        }
        "thismonth" => {
            let d = today.with_day(1).unwrap();
            let e = if d.month() == 12 {
                NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1)
            }
            .context("日期超出范围")?;
            (d, e)
        }
        _ => {
            let d =
                NaiveDate::parse_from_str(s, "%Y-%m-%d").context("日期无效，使用 YYYY-MM-DD")?;
            (d, d.succ_opt().context("日期超出范围")?)
        }
    };
    let convert = |d: NaiveDate| -> Result<u64> {
        let t = Local
            .from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .context("本地日期边界无效")?
            .timestamp();
        u64::try_from((i128::from(t) + 11644473600) * 10000000).context("日期超出范围")
    };
    Ok((convert(a)?, convert(b)? - 1))
}
fn bounds(s: &str, is_date: bool) -> Result<Bounds> {
    let value = |s: &str| -> Result<(u64, u64)> {
        if is_date {
            date(s)
        } else {
            let n = quantity(s)?;
            Ok((n, n))
        }
    };
    if let Some((a, b)) = s.split_once("..") {
        let min = value(a)?.0;
        let max = value(b)?.1;
        ensure!(min <= max, "区间下限大于上限");
        return Ok(Bounds {
            min: Some(min),
            max: Some(max),
        });
    }
    let (op, s) = if let Some(v) = s.strip_prefix(">=") {
        (">=", v)
    } else if let Some(v) = s.strip_prefix("<=") {
        ("<=", v)
    } else if let Some(v) = s.strip_prefix('>') {
        (">", v)
    } else if let Some(v) = s.strip_prefix('<') {
        ("<", v)
    } else {
        ("=", s.trim_start_matches('='))
    };
    let (a, b) = value(s)?;
    Ok(match op {
        ">=" => Bounds {
            min: Some(a),
            max: None,
        },
        ">" => Bounds {
            min: Some(b.checked_add(1).context("比较值超出范围")?),
            max: None,
        },
        "<=" => Bounds {
            min: None,
            max: Some(b),
        },
        "<" => Bounds {
            min: None,
            max: Some(a.checked_sub(1).context("比较值超出范围")?),
        },
        _ => Bounds {
            min: Some(a),
            max: Some(b),
        },
    })
}
