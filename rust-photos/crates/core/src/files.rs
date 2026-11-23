//! File operations: atomic writes, rotate, delete / restore, copy / move, rename, duplicates, wallpaper.
use crate::{decode, exif_util};
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Writes to a temporary file first, then swaps it in, so a crash never leaves a half written picture.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("fp-tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Rotates a picture on disk by 90, 180 or 270 degrees (clockwise). JPEG keeps its EXIF block.
pub fn rotate_file(path: &Path, deg: u32, jpeg_quality: u8) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let img = decode::open_upright(path)?;
    let img = match deg % 360 {
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => img,
    };
    let mut buf = Vec::new();
    match ext.as_str() {
        "jpg" | "jpeg" | "jfif" => {
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, jpeg_quality);
            image::DynamicImage::ImageRgb8(img.to_rgb8()).write_with_encoder(enc).map_err(|e| e.to_string())?;
            if let Ok(orig) = std::fs::read(path) {
                buf = exif_util::carry_exif(&orig, buf);
            }
        }
        "png" | "bmp" | "webp" => {
            let fmt = match ext.as_str() {
                "png" => image::ImageFormat::Png,
                "bmp" => image::ImageFormat::Bmp,
                _ => image::ImageFormat::WebP,
            };
            img.write_to(&mut std::io::Cursor::new(&mut buf), fmt).map_err(|e| e.to_string())?;
        }
        _ => return Err("unsupported".into()),
    }
    write_atomic(path, &buf)
}

/// Moves a picture to the Recycle Bin.
pub fn delete_file(path: &str) -> Result<(), String> {
    trash::delete(path).map_err(|e| e.to_string())
}

/// Puts the most recently deleted copy of a picture back where it was (Windows Recycle Bin).
pub fn restore_file(path: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut items: Vec<_> = trash::os_limited::list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|i| i.original_path().to_string_lossy().to_lowercase() == path.to_lowercase())
            .collect();
        items.sort_by_key(|i| i.time_deleted);
        let last = items.pop().ok_or("not in the Recycle Bin")?;
        trash::os_limited::restore_all([last]).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("unsupported".to_string())
    }
}

/// "name (2).ext" style name that does not exist yet.
pub fn unique(dest: PathBuf) -> PathBuf {
    if !dest.exists() {
        return dest;
    }
    let stem = dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = dest.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    let dir = dest.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for n in 2..10_000 {
        let c = dir.join(format!("{stem} ({n}){ext}"));
        if !c.exists() {
            return c;
        }
    }
    dest
}

/// Copies (or moves) pictures into a folder. Name clashes get " (2)", " (3)"... Returns how many succeeded.
pub fn transfer_files(paths: &[String], dest: &str, mv: bool) -> Result<usize, String> {
    let d = PathBuf::from(dest);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let mut ok = 0;
    for p in paths {
        let src = PathBuf::from(p);
        let Some(name) = src.file_name() else { continue };
        let target = unique(d.join(name));
        if target == src {
            continue;
        }
        let done = if mv {
            std::fs::rename(&src, &target).or_else(|_| std::fs::copy(&src, &target).and_then(|_| std::fs::remove_file(&src))).is_ok()
        } else {
            std::fs::copy(&src, &target).is_ok()
        };
        if done {
            ok += 1;
        }
    }
    Ok(ok)
}

/// Renames a picture inside its folder and returns the new path.
pub fn rename_file(path: &str, new_name: &str) -> Result<String, String> {
    let bad = |c: char| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
    let n = new_name.trim();
    if n.is_empty() || n.chars().any(bad) {
        return Err("invalid name".into());
    }
    let src = PathBuf::from(path);
    let target = src.with_file_name(n);
    if target == src {
        return Ok(path.to_string());
    }
    if target.exists() {
        return Err("exists".into());
    }
    std::fs::rename(&src, &target).map_err(|e| e.to_string())?;
    Ok(target.to_string_lossy().into_owned())
}

/// Result of the batch-rename pattern: `{n}` is the counter, `{name}` the original name without extension.
pub fn build_name(pattern: &str, original: &str, i: u32, digits: usize) -> String {
    let stem = original.rsplit_once('.').map(|(s, _)| s).unwrap_or(original);
    pattern.replace("{n}", &format!("{:0>w$}", i, w = digits)).replace("{name}", stem)
}

fn hash_file(p: &str) -> Option<u64> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.digest())
}

/// Groups byte-identical pictures. Only files with the same size are hashed, in parallel.
pub fn find_duplicates(paths: &[String]) -> Vec<Vec<String>> {
    let mut by_size: HashMap<u64, Vec<&String>> = HashMap::new();
    for p in paths {
        if let Ok(m) = std::fs::metadata(p) {
            by_size.entry(m.len()).or_default().push(p);
        }
    }
    let candidates: Vec<&String> = by_size.into_values().filter(|f| f.len() > 1).flatten().collect();
    let hashed: Vec<(u64, u64, &String)> = candidates
        .par_iter()
        .filter_map(|p| Some((std::fs::metadata(p).ok()?.len(), hash_file(p)?, *p)))
        .collect();
    let mut groups: HashMap<(u64, u64), Vec<String>> = HashMap::new();
    for (len, h, p) in hashed {
        groups.entry((len, h)).or_default().push(p.clone());
    }
    let mut out: Vec<Vec<String>> = groups.into_values().filter(|g| g.len() > 1).collect();
    for g in out.iter_mut() {
        g.sort();
    }
    out.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a[0].cmp(&b[0])));
    out
}

/// Sets the Windows desktop wallpaper (no PowerShell: one direct system call).
pub fn set_wallpaper(path: &str, tmp_dir: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_SETDESKWALLPAPER};
        let p = Path::new(path);
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        // Windows only accepts bmp / jpg / png here.
        let file = if matches!(ext.as_str(), "bmp" | "jpg" | "jpeg" | "png") {
            p.to_path_buf()
        } else {
            let img = decode::open_upright(p)?;
            std::fs::create_dir_all(tmp_dir).map_err(|e| e.to_string())?;
            let out = tmp_dir.join("wallpaper.png");
            img.save(&out).map_err(|e| e.to_string())?;
            out
        };
        let wide: Vec<u16> = file.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let ok = unsafe { SystemParametersInfoW(SPI_SETDESKWALLPAPER, 0, wide.as_ptr() as *mut _, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE) };
        if ok != 0 { Ok(()) } else { Err("failed".into()) }
    }
    #[cfg(not(windows))]
    {
        let _ = (path, tmp_dir);
        Err("unsupported".to_string())
    }
}
