//! Small helpers: pixel buffers, histogram paths, dates, and the few OS calls Rust's std does not cover.
use image::RgbaImage;
use slint::{Rgba8Pixel, SharedPixelBuffer};
use std::path::Path;

pub fn to_buf(img: &RgbaImage) -> SharedPixelBuffer<Rgba8Pixel> {
    SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height())
}

/// SVG-style path commands for a histogram, drawn in a 256 x 100 box.
pub fn hist_path(bins: &[u32; 256], max: u32) -> String {
    let max = max.max(1) as f32;
    let mut s = String::with_capacity(256 * 12);
    s.push_str("M 0 100");
    for (i, v) in bins.iter().enumerate() {
        // square-root scale keeps small tones visible
        let h = ((*v as f32 / max).sqrt() * 98.0).min(98.0);
        s.push_str(&format!(" L {} {:.1}", i, 100.0 - h));
    }
    s.push_str(" L 255 100 Z");
    s
}

pub fn fmt_time(ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms as i64).single().map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

pub fn base_name(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}

pub fn dir_name(p: &str) -> String {
    Path::new(p).parent().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Opens the file manager with the picture selected.
pub fn reveal(path: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.replace('/', "\\"))).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").args(["-R", path]).spawn();
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(dir_name(path)).spawn();
    }
}

/// Windows "Print pictures" dialog for one picture.
pub fn print_image(path: &str) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let w = |s: &str| -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect() };
        let (verb, file) = (w("print"), w(path));
        let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1) };
        (r as isize) > 32
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Copies a picture to the clipboard as an image.
pub fn copy_image(img: &RgbaImage) -> bool {
    match arboard::Clipboard::new() {
        Ok(mut c) => c.set_image(arboard::ImageData { width: img.width() as usize, height: img.height() as usize, bytes: std::borrow::Cow::Borrowed(img.as_raw()) }).is_ok(),
        Err(_) => false,
    }
}

pub fn copy_text(t: &str) -> bool {
    arboard::Clipboard::new().and_then(|mut c| c.set_text(t.to_string())).is_ok()
}

/// Runs decoding work so that a panic inside an image decoder (a corrupt file) fails that one picture instead of the app.
pub fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err("could not decode this file".into()))
}

/// Writes panics to `crash.log` in the config folder (the release build has no console to show them).
pub fn install_crash_log() {
    std::panic::set_hook(Box::new(|info| {
        use std::io::Write;
        let dir = rust_photos_core::store::config_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
            let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), info);
        }
    }));
}
