//! All file-system work. Everything here is plain Rust on std threads - the UI thread never
//! blocks on disk: the app calls these from worker threads and gets the result back by channel.

use crate::model::*;
use std::{
    collections::HashMap,
    fs,
    hash::Hasher,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/* ------------------------------------------------------------------ helpers */

#[allow(unused_mut)]
pub fn quiet(mut c: Command) -> Command {
    #[cfg(windows)]
    {
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

fn ms(t: std::io::Result<SystemTime>) -> i64 {
    t.ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(windows)]
fn is_hidden(_name: &str, md: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & 0x2 != 0
}
#[cfg(not(windows))]
fn is_hidden(name: &str, _md: &fs::Metadata) -> bool {
    name.starts_with('.')
}

pub fn make_entry(path: &Path, md: &fs::Metadata) -> Entry {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let is_dir = md.is_dir();
    let ext = if is_dir {
        String::new()
    } else {
        path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
    };
    Entry {
        hidden: is_hidden(&name, md),
        readonly: md.permissions().readonly(),
        is_dir,
        size: if is_dir { 0 } else { md.len() },
        modified: ms(md.modified()),
        created: ms(md.created()),
        name_lc: name.to_lowercase(),
        path: path.to_path_buf(),
        name,
        ext,
    }
}

pub fn split_name(name: &str, is_dir: bool) -> (String, String) {
    if is_dir {
        return (name.to_string(), String::new());
    }
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}

/// First free name in `dir`: "name (2)" or, with `copy_style`, "name - Copy".
pub fn free_name(dir: &Path, name: &str, is_dir: bool, copy_style: bool) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = split_name(name, is_dir);
    let mut i = 1u32;
    loop {
        let cand = if copy_style {
            if i == 1 {
                format!("{stem} - Copy{ext}")
            } else {
                format!("{stem} - Copy ({i}){ext}")
            }
        } else {
            format!("{stem} ({}){ext}", i + 1)
        };
        let p = dir.join(cand);
        if !p.exists() {
            return p;
        }
        i += 1;
    }
}

pub fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !(n.is_empty() || n == "." || n == ".." || n.chars().any(|c| "\\/:*?\"<>|".contains(c)))
}

/// Run `f` over `items` on all CPU cores and keep the order of the results.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(n).min(16);
    if workers <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(n));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let r = f(&items[i]);
                out.lock().unwrap().push((i, r));
            });
        }
    });
    let mut v = out.into_inner().unwrap();
    v.sort_by_key(|(i, _)| *i);
    v.into_iter().map(|(_, r)| r).collect()
}

/* --------------------------------------------------------------- listing */

/// Lists a folder. Metadata is read on several threads for big folders. Links are followed
/// only when needed, so a normal entry costs a single (usually free) metadata call.
pub fn list_dir(path: &Path) -> Result<Vec<Entry>, String> {
    let rd = fs::read_dir(path).map_err(|e| e.to_string())?;
    let items: Vec<fs::DirEntry> = rd.flatten().collect();
    let conv = |it: &fs::DirEntry| -> Option<Entry> {
        let p = it.path();
        let mut md = it.metadata().ok()?;
        if md.file_type().is_symlink() {
            // follow links / junctions so they behave like folders
            if let Ok(m) = fs::metadata(&p) {
                md = m;
            }
        }
        Some(make_entry(&p, &md))
    };
    if items.len() < 1500 {
        return Ok(items.iter().filter_map(conv).collect());
    }
    let chunks: Vec<&[fs::DirEntry]> = items.chunks(512).collect();
    let parts = par_map(&chunks, |c| c.iter().filter_map(conv).collect::<Vec<_>>());
    Ok(parts.into_iter().flatten().collect())
}

#[derive(Clone)]
pub struct Drive {
    pub name: String,
    pub path: PathBuf,
    pub total: u64,
    pub free: u64,
    pub fs: String,
}

pub fn get_drives() -> Vec<Drive> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut out: Vec<Drive> = Vec::new();
    for d in disks.list() {
        let path = d.mount_point().to_path_buf();
        if d.total_space() == 0 || out.iter().any(|x| x.path == path) {
            continue;
        }
        out.push(Drive {
            name: d.name().to_string_lossy().into_owned(),
            path,
            total: d.total_space(),
            free: d.available_space(),
            fs: d.file_system().to_string_lossy().into_owned(),
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

pub fn get_places() -> Vec<(&'static str, PathBuf)> {
    let c: Vec<(&'static str, Option<PathBuf>)> = vec![
        ("Home", dirs::home_dir()),
        ("Desktop", dirs::desktop_dir()),
        ("Documents", dirs::document_dir()),
        ("Downloads", dirs::download_dir()),
        ("Pictures", dirs::picture_dir()),
        ("Music", dirs::audio_dir()),
        ("Videos", dirs::video_dir()),
    ];
    c.into_iter().filter_map(|(n, p)| p.filter(|p| p.exists()).map(|p| (n, p))).collect()
}

/* ------------------------------------------------------- copy / move / delete */

/// Progress shared between the worker thread and the UI.
#[derive(Default)]
pub struct Shared {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
    pub name: Mutex<String>,
}

fn interrupted() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled")
}

pub fn tree_size(p: &Path) -> u64 {
    match fs::symlink_metadata(p) {
        Ok(md) if md.is_dir() => fs::read_dir(p).map(|rd| rd.flatten().map(|e| tree_size(&e.path())).sum()).unwrap_or(0),
        Ok(md) => md.len(),
        Err(_) => 0,
    }
}

pub fn dir_stats(p: &Path, files: &mut u64, folders: &mut u64, size: &mut u64) {
    if let Ok(rd) = fs::read_dir(p) {
        for item in rd.flatten() {
            if let Ok(md) = item.metadata() {
                if md.is_dir() {
                    *folders += 1;
                    dir_stats(&item.path(), files, folders, size);
                } else {
                    *files += 1;
                    *size += md.len();
                }
            }
        }
    }
}

fn copy_tracked(src: &Path, dst: &Path, sh: &Shared) -> std::io::Result<()> {
    if sh.cancel.load(Ordering::Relaxed) {
        return Err(interrupted());
    }
    if src.is_dir() {
        fs::create_dir(dst)?;
        for item in fs::read_dir(src)? {
            let item = item?;
            copy_tracked(&item.path(), &dst.join(item.file_name()), sh)?;
        }
        return Ok(());
    }
    if let Ok(mut n) = sh.name.lock() {
        *n = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    }
    let md = fs::metadata(src)?;
    if md.len() < 8 * 1024 * 1024 {
        // small files: the OS copy keeps timestamps and attributes
        fs::copy(src, dst)?;
        sh.done.fetch_add(md.len(), Ordering::Relaxed);
        return Ok(());
    }
    let mut r = fs::File::open(src)?;
    let mut w = fs::File::create(dst)?;
    let mut buf = vec![0u8; 2 << 20];
    loop {
        if sh.cancel.load(Ordering::Relaxed) {
            drop(w);
            let _ = fs::remove_file(dst);
            return Err(interrupted());
        }
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        sh.done.fetch_add(n as u64, Ordering::Relaxed);
    }
    if let Ok(modified) = md.modified() {
        let _ = w.set_times(fs::FileTimes::new().set_modified(modified));
    }
    let _ = fs::set_permissions(dst, md.permissions());
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Policy {
    Rename,
    Replace,
    Skip,
}

/// Names that already exist in `dest` (ignores pastes into the same folder).
pub fn check_conflicts(sources: &[PathBuf], dest: &Path) -> Vec<String> {
    sources
        .iter()
        .filter_map(|s| {
            let name = s.file_name()?.to_owned();
            if s.parent() == Some(dest) {
                return None;
            }
            dest.join(&name).exists().then(|| name.to_string_lossy().into_owned())
        })
        .collect()
}

/// Copy (`mv = false`) or move (`mv = true`) into `dest`. Returns `(source, new path)` pairs.
pub fn paste(sources: &[PathBuf], dest: &Path, mv: bool, policy: Policy, sh: &Shared) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    sh.cancel.store(false, Ordering::SeqCst);
    if !mv {
        sh.total.store(sources.iter().map(|s| tree_size(s)).sum(), Ordering::Relaxed);
    }
    let mut created = Vec::new();
    for src in sources {
        let Some(name) = src.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let is_dir = src.is_dir();
        if is_dir && dest.starts_with(src) {
            return Err("into-itself".into());
        }
        let same_parent = src.parent() == Some(dest);
        if mv && same_parent {
            continue;
        }
        let direct = dest.join(&name);
        let target = if !same_parent && direct.exists() {
            match policy {
                Policy::Skip => continue,
                Policy::Replace => {
                    if src.starts_with(&direct) {
                        return Err("into-itself".into());
                    }
                    let res = if direct.is_dir() { fs::remove_dir_all(&direct) } else { fs::remove_file(&direct) };
                    res.map_err(|e| e.to_string())?;
                    direct
                }
                Policy::Rename => free_name(dest, &name, is_dir, false),
            }
        } else {
            free_name(dest, &name, is_dir, same_parent)
        };
        let cleanup = |target: &Path| {
            let _ = if target.is_dir() { fs::remove_dir_all(target) } else { fs::remove_file(target) };
        };
        if mv {
            if fs::rename(src, &target).is_err() {
                // different volume: copy with progress, then remove the source
                sh.total.fetch_add(tree_size(src), Ordering::Relaxed);
                if let Err(e) = copy_tracked(src, &target, sh) {
                    cleanup(&target);
                    return Err(if sh.cancel.load(Ordering::Relaxed) { "cancelled".into() } else { e.to_string() });
                }
                let _ = if is_dir { fs::remove_dir_all(src) } else { fs::remove_file(src) };
            }
        } else if let Err(e) = copy_tracked(src, &target, sh) {
            cleanup(&target);
            return Err(if sh.cancel.load(Ordering::Relaxed) { "cancelled".into() } else { e.to_string() });
        }
        created.push((src.clone(), target));
    }
    Ok(created)
}

pub fn delete_items(paths: &[PathBuf], permanent: bool) -> Result<(), String> {
    if permanent {
        for p in paths {
            let res = if p.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) };
            res.map_err(|e| e.to_string())?;
        }
        Ok(())
    } else {
        trash::delete_all(paths).map_err(|e| e.to_string())
    }
}

pub fn create_item(dir: &Path, name: &str, is_dir: bool) -> Result<PathBuf, String> {
    let target = free_name(dir, name, is_dir, false);
    if is_dir {
        fs::create_dir(&target).map_err(|e| e.to_string())?;
    } else {
        fs::File::create(&target).map_err(|e| e.to_string())?;
    }
    Ok(target)
}

pub fn rename_item(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let name = new_name.trim();
    if !valid_name(name) {
        return Err("invalid-name".into());
    }
    let parent = path.parent().ok_or("invalid-path")?;
    let target = parent.join(name);
    let same_ignoring_case = path.file_name().map(|n| n.to_string_lossy().to_lowercase() == name.to_lowercase()).unwrap_or(false);
    if target.exists() && !same_ignoring_case {
        return Err("exists".into());
    }
    fs::rename(path, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

#[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))))]
pub fn restore_trashed(paths: &[PathBuf]) -> Result<usize, String> {
    let items = trash::os_limited::list().map_err(|e| e.to_string())?;
    let mut pick = Vec::new();
    for p in paths {
        let mut best: Option<trash::TrashItem> = None;
        for it in &items {
            if it.original_path() == *p && best.as_ref().map_or(true, |b| it.time_deleted > b.time_deleted) {
                best = Some(it.clone());
            }
        }
        if let Some(b) = best {
            pick.push(b);
        }
    }
    let n = pick.len();
    trash::os_limited::restore_all(pick).map_err(|e| e.to_string())?;
    Ok(n)
}
#[cfg(not(any(target_os = "windows", all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))))]
pub fn restore_trashed(_paths: &[PathBuf]) -> Result<usize, String> {
    Err("unsupported".into())
}

/* ------------------------------------------------------------- properties */

#[derive(Clone, Default)]
pub struct Props {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub files: u64,
    pub folders: u64,
    pub created: i64,
    pub modified: i64,
    pub accessed: i64,
    pub readonly: bool,
    pub hidden: bool,
}

pub fn item_properties(p: &Path) -> Result<Props, String> {
    let md = fs::metadata(p).map_err(|e| e.to_string())?;
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned());
    let (mut files, mut folders, mut size) = (0, 0, 0);
    if md.is_dir() {
        dir_stats(p, &mut files, &mut folders, &mut size);
    } else {
        size = md.len();
    }
    Ok(Props {
        hidden: is_hidden(&name, &md),
        readonly: md.permissions().readonly(),
        is_dir: md.is_dir(),
        created: ms(md.created()),
        modified: ms(md.modified()),
        accessed: ms(md.accessed()),
        name,
        path: p.to_path_buf(),
        size,
        files,
        folders,
    })
}

/// First few KB of a text file for the preview (error for binary files).
pub fn preview_text(p: &Path, max: usize) -> Result<String, String> {
    let mut f = fs::File::open(p).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(n);
    if buf.contains(&0) {
        return Err("binary".into());
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/* ------------------------------------------------------------------ search */

#[derive(Clone, Default)]
pub struct SearchOpts {
    pub regex: bool,
    pub exts: Vec<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub after: Option<i64>,
    pub before: Option<i64>,
}

/// Recursive name search. A newer search (`seq` != `id`) cancels this one.
pub fn search(root: &Path, query: &str, limit: usize, opts: &SearchOpts, id: u64, seq: &AtomicU64) -> Result<Vec<Entry>, String> {
    let re = if opts.regex && !query.is_empty() {
        Some(regex::RegexBuilder::new(query).case_insensitive(true).build().map_err(|_| "bad-regex".to_string())?)
    } else {
        None
    };
    let needle = query.to_lowercase();
    let file_filters = !opts.exts.is_empty() || opts.min_size.is_some() || opts.max_size.is_some();
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for item in rd.flatten() {
            visited += 1;
            if visited % 512 == 0 && seq.load(Ordering::Relaxed) != id {
                return Err("cancelled".into());
            }
            if visited > 600_000 || out.len() >= limit {
                return Ok(out);
            }
            let Ok(md) = item.metadata() else { continue };
            let p = item.path();
            let name = item.file_name().to_string_lossy().into_owned();
            let hit = match &re {
                Some(r) => r.is_match(&name),
                None => needle.is_empty() || name.to_lowercase().contains(&needle),
            };
            if hit {
                let mdf = fs::metadata(&p).unwrap_or_else(|_| md.clone());
                let is_dir = mdf.is_dir();
                let mut ok = !(is_dir && file_filters);
                if ok && !is_dir {
                    if !opts.exts.is_empty() {
                        let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                        ok = opts.exts.iter().any(|e| *e == ext);
                    }
                    let len = mdf.len();
                    ok = ok && opts.min_size.map_or(true, |m| len >= m) && opts.max_size.map_or(true, |m| len <= m);
                }
                if ok && (opts.after.is_some() || opts.before.is_some()) {
                    let m = ms(mdf.modified());
                    ok = opts.after.map_or(true, |a| m >= a) && opts.before.map_or(true, |b| m <= b);
                }
                if ok {
                    out.push(make_entry(&p, &mdf));
                }
            }
            if md.is_dir() && !md.file_type().is_symlink() {
                stack.push(p);
            }
        }
    }
    Ok(out)
}

/* --------------------------------------------------------- duplicate finder */

#[derive(Clone)]
pub struct DupGroup {
    pub size: u64,
    pub files: Vec<PathBuf>,
}

fn hash_file(p: &Path, limit: Option<u64>, cancel: &AtomicBool) -> Option<u128> {
    let mut f = fs::File::open(p).ok()?;
    let mut h1 = std::collections::hash_map::DefaultHasher::new();
    let mut h2 = std::collections::hash_map::DefaultHasher::new();
    h2.write_u8(0xA5);
    let mut buf = vec![0u8; 256 * 1024];
    let mut left = limit.unwrap_or(u64::MAX);
    while left > 0 {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let want = buf.len().min(left.min(usize::MAX as u64) as usize);
        let n = f.read(&mut buf[..want]).ok()?;
        if n == 0 {
            break;
        }
        h1.write(&buf[..n]);
        h2.write(&buf[..n]);
        left -= n as u64;
    }
    Some(((h1.finish() as u128) << 64) | h2.finish() as u128)
}

/// Finds identical files below `root`: group by size, then a quick hash of the first 64 KB,
/// then a full hash - the hashing runs on all CPU cores.
pub fn find_duplicates(root: &Path, min_size: u64, cancel: &AtomicBool) -> Vec<DupGroup> {
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for it in rd.flatten() {
            let Ok(md) = it.metadata() else { continue };
            if md.file_type().is_symlink() {
                continue;
            }
            if md.is_dir() {
                stack.push(it.path());
            } else if md.len() >= min_size.max(1) {
                by_size.entry(md.len()).or_default().push(it.path());
            }
        }
    }
    let mut groups: Vec<(u64, Vec<PathBuf>)> = by_size.into_iter().filter(|(_, v)| v.len() > 1).collect();
    let mut result = Vec::new();
    for pass in 0..2 {
        let mut next: Vec<(u64, Vec<PathBuf>)> = Vec::new();
        for (size, files) in groups {
            if cancel.load(Ordering::Relaxed) {
                return Vec::new();
            }
            let limit = if pass == 0 { Some(64 * 1024) } else { None };
            // small files are fully hashed in the first pass already
            let hashes = par_map(&files, |p| hash_file(p, limit, cancel));
            let mut m: HashMap<u128, Vec<PathBuf>> = HashMap::new();
            for (p, h) in files.into_iter().zip(hashes) {
                if let Some(h) = h {
                    m.entry(h).or_default().push(p);
                }
            }
            for (_, v) in m {
                if v.len() > 1 {
                    if pass == 0 && size > 64 * 1024 {
                        next.push((size, v));
                    } else {
                        result.push(DupGroup { size, files: v });
                    }
                }
            }
        }
        groups = next;
    }
    result.sort_by(|a, b| (b.size * b.files.len() as u64).cmp(&(a.size * a.files.len() as u64)));
    result
}

/* ------------------------------------------------------------- disk usage */

#[derive(Clone)]
pub struct SizeItem {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
}

fn walk_size(p: &Path, cancel: &AtomicBool, deadline: Instant) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![p.to_path_buf()];
    let mut n = 0u32;
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for it in rd.flatten() {
            n += 1;
            if n % 256 == 0 && (cancel.load(Ordering::Relaxed) || Instant::now() > deadline) {
                return total;
            }
            let Ok(md) = it.metadata() else { continue };
            if md.file_type().is_symlink() {
                continue;
            }
            if md.is_dir() {
                stack.push(it.path());
            } else {
                total += md.len();
            }
        }
    }
    total
}

/// Sizes of the direct children of `path`, computed in parallel, biggest first.
pub fn folder_sizes(path: &Path, cancel: &AtomicBool) -> Result<Vec<SizeItem>, String> {
    let rd = fs::read_dir(path).map_err(|e| e.to_string())?;
    let kids: Vec<(PathBuf, bool, u64)> = rd
        .flatten()
        .filter_map(|it| {
            let md = it.metadata().ok()?;
            if md.file_type().is_symlink() {
                return None;
            }
            Some((it.path(), md.is_dir(), md.len()))
        })
        .collect();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut v = par_map(&kids, |(p, is_dir, len)| SizeItem {
        name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        path: p.clone(),
        size: if *is_dir { walk_size(p, cancel, deadline) } else { *len },
        is_dir: *is_dir,
    });
    v.retain(|i| i.size > 0);
    v.sort_by(|a, b| b.size.cmp(&a.size));
    Ok(v)
}

/// Size of one folder, used by the optional "folder sizes" column.
pub fn quick_dir_size(p: &Path) -> u64 {
    let never = AtomicBool::new(false);
    walk_size(p, &never, Instant::now() + Duration::from_secs(20))
}

/* ----------------------------------------------------------------- archives */

pub fn compress_zip(paths: &[PathBuf], out: &Path) -> Result<(), String> {
    use zip::write::SimpleFileOptions;
    let f = fs::File::create(out).map_err(|e| e.to_string())?;
    let mut zw = zip::ZipWriter::new(std::io::BufWriter::new(f));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    fn add(zw: &mut zip::ZipWriter<std::io::BufWriter<fs::File>>, root: &Path, p: &Path, opts: SimpleFileOptions) -> Result<(), String> {
        let rel = p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
        if p.is_dir() {
            zw.add_directory(format!("{rel}/"), opts).map_err(|e| e.to_string())?;
            for it in fs::read_dir(p).map_err(|e| e.to_string())?.flatten() {
                add(zw, root, &it.path(), opts)?;
            }
        } else {
            zw.start_file(rel, opts).map_err(|e| e.to_string())?;
            let mut f = fs::File::open(p).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, zw).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    for p in paths {
        let root = p.parent().unwrap_or(Path::new(""));
        add(&mut zw, root, p, opts)?;
    }
    zw.finish().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn extract_zip(src: &Path, out_dir: &Path) -> Result<(), String> {
    let f = fs::File::open(src).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| e.to_string())?;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name rejects "../" tricks (zip-slip)
        let Some(rel) = e.enclosed_name() else { continue };
        let target = out_dir.join(rel);
        if e.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else {
            if let Some(p) = target.parent() {
                fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let mut w = fs::File::create(&target).map_err(|e| e.to_string())?;
            std::io::copy(&mut e, &mut w).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn program_files() -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"].iter().filter_map(|k| std::env::var_os(k)).map(PathBuf::from).collect()
}

pub fn on_path(name: &str) -> bool {
    let finder = if cfg!(windows) { "where" } else { "which" };
    quiet(Command::new(finder)).arg(name).output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn find_winrar() -> Option<PathBuf> {
    for b in program_files() {
        let p = b.join("WinRAR").join("WinRAR.exe");
        if p.exists() {
            return Some(p);
        }
    }
    on_path("WinRAR").then(|| PathBuf::from("WinRAR"))
}

pub fn find_7z() -> Option<PathBuf> {
    for b in program_files() {
        let p = b.join("7-Zip").join("7z.exe");
        if p.exists() {
            return Some(p);
        }
    }
    on_path("7z").then(|| PathBuf::from("7z"))
}

fn run_ok(mut cmd: Command, ok: &[i32]) -> bool {
    cmd.status().map(|s| s.code().map(|c| ok.contains(&c)).unwrap_or(false)).unwrap_or(false)
}

/// ZIP is extracted natively in Rust. Other formats use WinRAR / 7-Zip / tar when installed.
pub fn extract_archive(path: &Path, dest: &Path, to_folder: bool) -> Result<PathBuf, String> {
    let mut stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "extracted".into());
    if stem.to_lowercase().ends_with(".tar") {
        stem.truncate(stem.len() - 4);
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let out_dir = if to_folder { free_name(dest, &stem, true, false) } else { dest.to_path_buf() };
    fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let mut ok = false;
    if ext == "zip" {
        match extract_zip(path, &out_dir) {
            Ok(()) => ok = true,
            Err(e) => {
                if find_winrar().is_none() && find_7z().is_none() {
                    if to_folder {
                        let _ = fs::remove_dir_all(&out_dir);
                    }
                    return Err(e);
                }
            }
        }
    }
    if !ok {
        if let Some(w) = find_winrar() {
            let sep = std::path::MAIN_SEPARATOR;
            let dest_arg = format!("{}{}", out_dir.to_string_lossy().trim_end_matches(['\\', '/']), sep);
            let mut c = Command::new(w);
            c.args(["x", "-ibck", "-y", "-o+"]).arg(path).arg(dest_arg);
            ok = run_ok(c, &[0, 1]);
        }
    }
    if !ok {
        if let Some(z) = find_7z() {
            let mut c = quiet(Command::new(z));
            c.args(["x", "-y"]).arg(format!("-o{}", out_dir.to_string_lossy())).arg(path);
            ok = run_ok(c, &[0, 1]);
        }
    }
    if !ok {
        let mut c = quiet(Command::new("tar"));
        c.arg("-xf").arg(path).arg("-C").arg(&out_dir);
        ok = run_ok(c, &[0]);
    }
    if ok {
        Ok(out_dir)
    } else {
        if to_folder {
            let _ = fs::remove_dir(&out_dir);
        }
        Err("no-extractor".into())
    }
}

/// `kind` is "zip" (native) or "rar" (needs WinRAR).
pub fn compress_items(paths: &[PathBuf], dest_dir: &Path, name: &str, kind: &str) -> Result<PathBuf, String> {
    if paths.is_empty() {
        return Err("empty".into());
    }
    let out = free_name(dest_dir, &format!("{name}.{kind}"), false, false);
    if kind == "zip" {
        return match compress_zip(paths, &out) {
            Ok(()) => Ok(out),
            Err(e) => {
                let _ = fs::remove_file(&out);
                Err(e)
            }
        };
    }
    let Some(w) = find_winrar() else { return Err("no-winrar".into()) };
    let mut c = Command::new(w);
    c.args(["a", "-ibck", "-ep1", "-y"]).arg(&out);
    for p in paths {
        c.arg(p);
    }
    if run_ok(c, &[0, 1]) {
        Ok(out)
    } else {
        Err("compress-failed".into())
    }
}

/* --------------------------------------------------------- open / tools */

pub fn reveal_item(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let parent = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        open::that(parent).map_err(|e| e.to_string())
    }
}

/// `tool`: terminal | powershell | code | notepad | winrar
pub fn open_with(tool: &str, path: &Path) -> Result<(), String> {
    let dir = if path.is_dir() { path.to_path_buf() } else { path.parent().map(|x| x.to_path_buf()).unwrap_or_else(|| path.to_path_buf()) };
    let dir_s = dir.to_string_lossy().into_owned();
    let spawn = |mut c: Command| c.spawn().map(|_| ()).map_err(|e| e.to_string());
    match tool {
        "terminal" => {
            #[cfg(windows)]
            {
                if quiet(Command::new("wt")).args(["-d", &dir_s]).spawn().is_ok() {
                    return Ok(());
                }
                return spawn(quiet({
                    let mut c = Command::new("cmd");
                    c.args(["/C", "start", "", "/D", &dir_s, "cmd.exe"]);
                    c
                }));
            }
            #[cfg(not(windows))]
            {
                for t in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
                    let mut c = Command::new(t);
                    c.current_dir(&dir);
                    if c.spawn().is_ok() {
                        return Ok(());
                    }
                }
                Err("not-found".into())
            }
        }
        "powershell" => spawn(quiet({
            let mut c = Command::new("cmd");
            c.args(["/C", "start", "", "/D", &dir_s, "powershell.exe"]);
            c
        })),
        "code" => {
            let mut c = quiet(Command::new(if cfg!(windows) { "cmd" } else { "code" }));
            if cfg!(windows) {
                c.args(["/C", "code"]);
            }
            c.arg(path);
            if run_ok(c, &[0]) {
                Ok(())
            } else {
                Err("not-found".into())
            }
        }
        "notepad" => spawn({
            let mut c = Command::new("notepad.exe");
            c.arg(path);
            c
        }),
        "winrar" => match find_winrar() {
            Some(w) => spawn({
                let mut c = Command::new(w);
                c.arg(path);
                c
            }),
            None => Err("no-winrar".into()),
        },
        _ => Err("unknown-tool".into()),
    }
}

pub fn run_custom(program: &str, args: &[String]) -> Result<(), String> {
    if program.trim().is_empty() {
        return Err("empty-program".into());
    }
    let mut c = quiet(Command::new(program.trim()));
    c.args(args);
    c.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Opens a built-in Windows tool.
pub fn run_system(kind: &str) -> Result<(), String> {
    let (prog, args): (&str, &[&str]) = match kind {
        "control" => ("control.exe", &[]),
        "settings" => ("explorer.exe", &["ms-settings:"]),
        "taskmgr" => ("taskmgr.exe", &[]),
        "devmgmt" => ("mmc.exe", &["devmgmt.msc"]),
        "diskmgmt" => ("mmc.exe", &["diskmgmt.msc"]),
        "services" => ("mmc.exe", &["services.msc"]),
        "eventvwr" => ("mmc.exe", &["eventvwr.msc"]),
        "cleanmgr" => ("cleanmgr.exe", &[]),
        "sysinfo" => ("msinfo32.exe", &[]),
        "envvars" => ("rundll32.exe", &["sysdm.cpl,EditEnvironmentVariables"]),
        "network" => ("control.exe", &["ncpa.cpl"]),
        "programs" => ("control.exe", &["appwiz.cpl"]),
        "regedit" => ("regedit.exe", &[]),
        _ => return Err("unknown-tool".into()),
    };
    if !cfg!(windows) {
        return Err("unsupported".into());
    }
    Command::new(prog).args(args).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[derive(Clone, Default)]
pub struct CleanResult {
    pub freed: u64,
    pub files: u32,
    pub skipped: u32,
}

fn clean_dir(dir: &Path, cutoff: SystemTime, r: &mut CleanResult, depth: u32) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).map(|t| t < cutoff).unwrap_or(false);
        if ft.is_dir() {
            if depth < 8 {
                clean_dir(&p, cutoff, r, depth + 1);
                if old {
                    let _ = fs::remove_dir(&p); // only succeeds when empty
                }
            }
        } else if old {
            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
            if fs::remove_file(&p).is_ok() {
                r.freed += len;
                r.files += 1;
            } else {
                r.skipped += 1;
            }
        }
    }
}

/// Deletes temp files untouched for a day (files in use are skipped).
pub fn clean_temp() -> CleanResult {
    let cutoff = SystemTime::now() - Duration::from_secs(24 * 3600);
    let mut r = CleanResult::default();
    clean_dir(&std::env::temp_dir(), cutoff, &mut r, 0);
    r
}

/// The Windows accent colour, read from the registry.
pub fn accent_color() -> Option<[u8; 3]> {
    #[cfg(windows)]
    {
        let out = Command::new("reg")
            .args(["query", r"HKCU\Software\Microsoft\Windows\DWM", "/v", "AccentColor"])
            .creation_flags(0x0800_0000)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let hex = text.split_whitespace().find(|w| w.starts_with("0x"))?;
        let v = u32::from_str_radix(&hex[2..], 16).ok()?;
        // DWORD is 0xAABBGGRR
        return Some([(v & 0xff) as u8, ((v >> 8) & 0xff) as u8, ((v >> 16) & 0xff) as u8]);
    }
    #[allow(unreachable_code)]
    None
}

pub fn set_wallpaper(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "user32")]
        extern "system" {
            fn SystemParametersInfoW(action: u32, param: u32, pv: *mut std::ffi::c_void, flags: u32) -> i32;
        }
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SPI_SETDESKWALLPAPER, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE
        let ok = unsafe { SystemParametersInfoW(0x0014, 0, wide.as_mut_ptr() as *mut std::ffi::c_void, 0x03) };
        if ok != 0 {
            Ok(())
        } else {
            Err("wallpaper-failed".into())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("unsupported".into())
    }
}

/* ---------------------------------------------------------------- thumbnails */

pub struct Rgba {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// Decodes an image and scales it down to fit `px`. Refuses huge files to protect memory.
pub fn make_thumb(path: &Path, px: u32) -> Option<Rgba> {
    let md = fs::metadata(path).ok()?;
    if md.len() > 60 * 1024 * 1024 {
        return None;
    }
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let t = img.thumbnail(px, px).to_rgba8();
    Some(Rgba { w: t.width() as usize, h: t.height() as usize, px: t.into_raw() })
}

/// Full-size decode for quick look (capped so a 100 MP photo cannot eat all memory).
pub fn load_image_capped(path: &Path, max_side: u32) -> Option<Rgba> {
    let md = fs::metadata(path).ok()?;
    if md.len() > 120 * 1024 * 1024 {
        return None;
    }
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let img = if img.width().max(img.height()) > max_side { img.thumbnail(max_side, max_side) } else { img };
    let t = img.to_rgba8();
    Some(Rgba { w: t.width() as usize, h: t.height() as usize, px: t.into_raw() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rustfile-test-{}-{}", name, now_ms()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copy_move_conflict_and_policy() {
        let root = tmp("paste");
        let (a, b) = (root.join("a"), root.join("b"));
        fs::create_dir_all(a.join("sub")).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "one").unwrap();
        fs::write(a.join("sub/y.txt"), "two").unwrap();
        fs::write(b.join("x.txt"), "old").unwrap();
        let sh = Shared::default();
        let src = vec![a.join("x.txt"), a.join("sub")];
        assert_eq!(check_conflicts(&src, &b), vec!["x.txt".to_string()]);
        let r = paste(&src, &b, false, Policy::Rename, &sh).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "old");
        assert_eq!(fs::read_to_string(b.join("x (2).txt")).unwrap(), "one");
        assert_eq!(fs::read_to_string(b.join("sub/y.txt")).unwrap(), "two");
        assert_eq!(sh.done.load(Ordering::Relaxed), 6);
        paste(&[a.join("x.txt")], &b, true, Policy::Replace, &sh).unwrap();
        assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "one");
        assert!(!a.join("x.txt").exists());
        assert_eq!(paste(&[a.clone()], &a.join("sub"), false, Policy::Rename, &sh).unwrap_err(), "into-itself");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_roundtrip() {
        let root = tmp("zip");
        fs::create_dir_all(root.join("d/inner")).unwrap();
        fs::write(root.join("d/inner/f.txt"), "hello").unwrap();
        fs::write(root.join("top.txt"), "top").unwrap();
        let out = compress_items(&[root.join("d"), root.join("top.txt")], &root, "pack", "zip").unwrap();
        let ex = extract_archive(&out, &root, true).unwrap();
        assert_eq!(fs::read_to_string(ex.join("d/inner/f.txt")).unwrap(), "hello");
        assert_eq!(fs::read_to_string(ex.join("top.txt")).unwrap(), "top");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicates_search_and_sizes() {
        let root = tmp("dup");
        fs::create_dir_all(root.join("s")).unwrap();
        let big = vec![7u8; 200_000];
        fs::write(root.join("a.bin"), &big).unwrap();
        fs::write(root.join("s/b.bin"), &big).unwrap();
        let mut other = big.clone();
        other[199_999] = 8; // same size, differs after the 64 KB quick hash
        fs::write(root.join("c.bin"), &other).unwrap();
        fs::write(root.join("note.txt"), "abc").unwrap();
        let g = find_duplicates(&root, 0, &AtomicBool::new(false));
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].files.len(), 2);
        let seq = AtomicU64::new(1);
        let hits = search(&root, "BIN", 100, &SearchOpts::default(), 1, &seq).unwrap();
        assert_eq!(hits.len(), 3);
        let re = SearchOpts { regex: true, ..Default::default() };
        assert_eq!(search(&root, r"^[ab]\.bin$", 100, &re, 1, &seq).unwrap().len(), 2);
        let sz = folder_sizes(&root, &AtomicBool::new(false)).unwrap();
        assert_eq!(sz[0].size, 200_000);
        assert_eq!(list_dir(&root).unwrap().len(), 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rename_rules_and_free_name() {
        let root = tmp("ren");
        fs::write(root.join("a.txt"), "1").unwrap();
        fs::write(root.join("b.txt"), "2").unwrap();
        assert_eq!(rename_item(&root.join("a.txt"), "b.txt").unwrap_err(), "exists");
        assert_eq!(rename_item(&root.join("a.txt"), "x/y").unwrap_err(), "invalid-name");
        assert_eq!(free_name(&root, "b.txt", false, false), root.join("b (2).txt"));
        assert_eq!(free_name(&root, "b.txt", false, true), root.join("b - Copy.txt"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn natural_sort() {
        use std::cmp::Ordering::*;
        assert_eq!(natural_cmp("note2", "note10"), Less);
        assert_eq!(natural_cmp("file01", "file1"), Equal);
    }
}
