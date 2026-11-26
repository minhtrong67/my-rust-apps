//! Plain data types and small helpers shared by the whole app.

use std::{cmp::Ordering, path::PathBuf};

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub name_lc: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    /// milliseconds since the Unix epoch, 0 = unknown
    pub modified: i64,
    pub created: i64,
    pub hidden: bool,
    pub readonly: bool,
    /// lower-case extension without the dot ("" for folders)
    pub ext: String,
}

impl Entry {
    pub fn kind(&self) -> Kind {
        if self.is_dir {
            Kind::Folder
        } else {
            kind_of(&self.ext)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Folder,
    Image,
    Video,
    Audio,
    Document,
    Archive,
    Code,
    Text,
    Other,
}

pub fn kind_of(ext: &str) -> Kind {
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "tif" | "tiff" | "heic" | "avif" => Kind::Image,
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "flv" | "m4v" => Kind::Video,
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" => Kind::Audio,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "rtf" | "epub" => Kind::Document,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "iso" => Kind::Archive,
        "rs" | "js" | "ts" | "tsx" | "jsx" | "py" | "c" | "h" | "cpp" | "hpp" | "cs" | "java" | "go" | "html" | "css" | "json"
        | "toml" | "yaml" | "yml" | "xml" | "sh" | "bat" | "ps1" | "sql" | "php" | "rb" | "kt" | "swift" | "lua" | "vue" => Kind::Code,
        "txt" | "md" | "log" | "ini" | "cfg" | "csv" | "tsv" | "conf" | "markdown" => Kind::Text,
        _ => Kind::Other,
    }
}

pub fn is_image_ext(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
}

pub fn is_text_ext(ext: &str) -> bool {
    matches!(kind_of(ext), Kind::Code | Kind::Text)
}

pub fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Folder => "Folder",
        Kind::Image => "Image",
        Kind::Video => "Video",
        Kind::Audio => "Audio",
        Kind::Document => "Document",
        Kind::Archive => "Archive",
        Kind::Code => "Source code",
        Kind::Text => "Text file",
        Kind::Other => "File",
    }
}

/// Case-insensitive "natural" comparison: `file2` sorts before `file10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (mut na, mut nb) = (String::new(), String::new());
                while let Some(&c) = ai.peek() {
                    if c.is_ascii_digit() {
                        na.push(c);
                        ai.next();
                    } else {
                        break;
                    }
                }
                while let Some(&c) = bi.peek() {
                    if c.is_ascii_digit() {
                        nb.push(c);
                        bi.next();
                    } else {
                        break;
                    }
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                match ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

pub fn fmt_size(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if b < 1024 {
        return format!("{b} B");
    }
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 {
        format!("{:.0} {}", v, U[i])
    } else if v >= 10.0 {
        format!("{:.1} {}", v, U[i])
    } else {
        format!("{:.2} {}", v, U[i])
    }
}

pub fn fmt_date(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    if ms <= 0 {
        return String::new();
    }
    match Local.timestamp_millis_opt(ms).single() {
        Some(d) => d.format("%d/%m/%Y %H:%M").to_string(),
        None => String::new(),
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Tiny xorshift RNG (no extra crate needed) for "random rename".
pub struct Rng(u64);
impl Rng {
    pub fn new() -> Self {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(88172645463325252);
        Rng(t | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn pick(&mut self, set: &str) -> char {
        let v: Vec<char> = set.chars().collect();
        v[(self.next() % v.len() as u64) as usize]
    }
}
