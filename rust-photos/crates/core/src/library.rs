//! Folder scanning, sorting and grouping of the picture list.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const EXTS: [&str; 10] = ["png", "jpg", "jpeg", "jfif", "gif", "webp", "bmp", "ico", "avif", "svg"];

pub fn is_img(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| EXTS.contains(&e.to_lowercase().as_str())).unwrap_or(false)
}

fn ms(m: &std::fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mtime: u64,
    pub folder: String,
}

#[derive(Clone, Debug)]
pub struct Opened {
    pub dir: String,
    pub entries: Vec<Entry>,
    pub current: Option<String>,
}

fn entry(p: &Path, m: &std::fs::Metadata) -> Option<Entry> {
    Some(Entry {
        path: p.to_string_lossy().into_owned(),
        name: p.file_name()?.to_string_lossy().into_owned(),
        size: m.len(),
        mtime: ms(m),
        folder: p.parent()?.to_string_lossy().into_owned(),
    })
}

/// Pictures directly inside `dir`.
pub fn scan(dir: &Path) -> Vec<Entry> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if is_img(&p) {
                if let Ok(m) = e.metadata() {
                    if m.is_file() {
                        v.extend(entry(&p, &m));
                    }
                }
            }
        }
    }
    v
}

/// Scans a folder and all of its sub folders (up to 32 levels deep, 50 000 pictures).
pub fn scan_deep(root: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.starts_with('$') {
                continue;
            }
            let p = e.path();
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                if depth < 32 {
                    stack.push((p, depth + 1));
                }
            } else if is_img(&p) {
                out.extend(entry(&p, &m));
                if out.len() >= 50_000 {
                    return out;
                }
            }
        }
    }
    out
}

/// The user's Pictures folder, or the home folder when Windows does not report one.
pub fn default_library() -> PathBuf {
    dirs::picture_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."))
}

pub fn library(path: &str) -> Result<Opened, String> {
    let dir = if path.trim().is_empty() { default_library() } else { PathBuf::from(path.trim()) };
    if !dir.is_dir() {
        return Err("not found".into());
    }
    Ok(Opened { dir: dir.to_string_lossy().into_owned(), entries: scan_deep(&dir), current: None })
}

/// Opens a folder, or the folder that contains the given picture.
pub fn open_target(path: &str) -> Result<Opened, String> {
    let p = PathBuf::from(path);
    if p.is_dir() {
        Ok(Opened { dir: path.into(), entries: scan(&p), current: None })
    } else if p.is_file() {
        let dir = p.parent().ok_or("no parent")?.to_path_buf();
        Ok(Opened { dir: dir.to_string_lossy().into_owned(), entries: scan(&dir), current: Some(path.into()) })
    } else {
        Err("not found".into())
    }
}

/// Size and date for a list of paths (favourites and albums can live in any folder).
pub fn stat_paths(paths: &[String]) -> Vec<Entry> {
    paths
        .iter()
        .filter_map(|p| {
            let pb = PathBuf::from(p);
            let m = std::fs::metadata(&pb).ok()?;
            if !m.is_file() || !is_img(&pb) {
                return None;
            }
            entry(&pb, &m)
        })
        .collect()
}

/// A picture or folder passed on the command line when Windows opens a file with this app.
pub fn arg_target(args: &[String]) -> Option<String> {
    args.iter().skip(1).find(|a| {
        let p = Path::new(a);
        p.is_dir() || (p.is_file() && is_img(p))
    }).cloned()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum SortKey {
    #[default]
    #[serde(rename = "date-desc")]
    DateDesc,
    #[serde(rename = "date-asc")]
    DateAsc,
    #[serde(rename = "name-asc")]
    NameAsc,
    #[serde(rename = "name-desc")]
    NameDesc,
    #[serde(rename = "size-desc")]
    SizeDesc,
    #[serde(rename = "size-asc")]
    SizeAsc,
}

impl SortKey {
    pub const ALL: [SortKey; 6] = [Self::DateDesc, Self::DateAsc, Self::NameAsc, Self::NameDesc, Self::SizeDesc, Self::SizeAsc];
    pub fn id(self) -> &'static str {
        match self {
            Self::DateDesc => "date-desc",
            Self::DateAsc => "date-asc",
            Self::NameAsc => "name-asc",
            Self::NameDesc => "name-desc",
            Self::SizeDesc => "size-desc",
            Self::SizeAsc => "size-asc",
        }
    }
}

/// Natural, case-insensitive name order ("img2" before "img10").
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut ia, mut ib) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ia.peek().copied().filter(|c| c.is_ascii_digit()) { na.push(c); ia.next(); }
                let mut nb = String::new();
                while let Some(c) = ib.peek().copied().filter(|c| c.is_ascii_digit()) { nb.push(c); ib.next(); }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != std::cmp::Ordering::Equal { return o; }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != std::cmp::Ordering::Equal { return o; }
                ia.next();
                ib.next();
            }
        }
    }
}

pub fn sort_entries(list: &mut [Entry], key: SortKey) {
    match key {
        SortKey::DateDesc => list.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| natural_cmp(&a.name, &b.name))),
        SortKey::DateAsc => list.sort_by(|a, b| a.mtime.cmp(&b.mtime).then_with(|| natural_cmp(&a.name, &b.name))),
        SortKey::NameAsc => list.sort_by(|a, b| natural_cmp(&a.name, &b.name)),
        SortKey::NameDesc => list.sort_by(|a, b| natural_cmp(&b.name, &a.name)),
        SortKey::SizeDesc => list.sort_by(|a, b| b.size.cmp(&a.size)),
        SortKey::SizeAsc => list.sort_by(|a, b| a.size.cmp(&b.size)),
    }
}

/// Day-group heading for a timestamp: Today, Yesterday, This week, This month, then "Month Year".
pub fn group_label(mtime_ms: u64, lang: crate::i18n::Lang, today: chrono::NaiveDate) -> String {
    use chrono::{Datelike, TimeZone};
    let Some(dt) = chrono::Local.timestamp_millis_opt(mtime_ms as i64).single() else { return String::new() };
    let d = dt.date_naive();
    let diff = (today - d).num_days();
    let tr = |k| crate::i18n::tr(lang, k);
    if diff <= 0 { return tr("date.today"); }
    if diff == 1 { return tr("date.yesterday"); }
    if diff < 7 { return tr("date.week"); }
    if d.year() == today.year() && d.month() == today.month() { return tr("date.month"); }
    const EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    match lang {
        crate::i18n::Lang::Vi => format!("tháng {} năm {}", d.month(), d.year()),
        crate::i18n::Lang::En => format!("{} {}", EN[d.month0() as usize], d.year()),
    }
}

/// Human readable file size.
pub fn fmt_size(n: u64) -> String {
    if n < 1024 { format!("{n} B") } else if n < 1_048_576 { format!("{} KB", n / 1024) } else { format!("{:.1} MB", n as f64 / 1_048_576.0) }
}
