//! Thumbnail cache: square JPEGs on disk, decoded in parallel on demand.
use crate::decode;
use fast_image_resize as fr;
use image::{RgbImage, RgbaImage};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const THUMB_SIZE: u32 = 256;

/// Resizes with a high quality filter (SIMD, alpha aware).
pub fn resize_to(src: &RgbaImage, dw: u32, dh: u32, filter: fr::FilterType) -> RgbaImage {
    let (dw, dh) = (dw.max(1), dh.max(1));
    let Ok(s) = fr::images::ImageRef::new(src.width(), src.height(), src.as_raw(), fr::PixelType::U8x4) else { return src.clone() };
    let mut d = fr::images::Image::new(dw, dh, fr::PixelType::U8x4);
    let opts = fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(filter));
    if fr::Resizer::new().resize(&s, &mut d, &opts).is_err() {
        return src.clone();
    }
    RgbaImage::from_raw(dw, dh, d.into_vec()).unwrap_or_else(|| src.clone())
}

/// Scales so the longest side is at most `max_side` (never enlarges).
pub fn resize_rgba(src: &RgbaImage, max_side: u32) -> RgbaImage {
    let (w, h) = src.dimensions();
    let k = max_side as f32 / w.max(h) as f32;
    if k >= 1.0 {
        return src.clone();
    }
    resize_to(src, (w as f32 * k).round() as u32, (h as f32 * k).round() as u32, fr::FilterType::Lanczos3)
}

/// Same as `fill_square` for pictures without transparency (3 bytes per pixel instead of 4).
pub fn fill_square_rgb(src: &RgbImage, size: u32) -> RgbImage {
    let (w, h) = src.dimensions();
    let side = w.min(h);
    let (left, top) = ((w - side) / 2, (h - side) / 2);
    let Ok(s) = fr::images::ImageRef::new(w, h, src.as_raw(), fr::PixelType::U8x3) else { return src.clone() };
    let mut d = fr::images::Image::new(size, size, fr::PixelType::U8x3);
    let opts = fr::ResizeOptions::new()
        .resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Bilinear))
        .crop(left as f64, top as f64, side as f64, side as f64);
    if fr::Resizer::new().resize(&s, &mut d, &opts).is_err() {
        return src.clone();
    }
    RgbImage::from_raw(size, size, d.into_vec()).unwrap_or_else(|| src.clone())
}

/// Centre-crops to a square and resizes to `size` x `size` (like CSS object-fit: cover).
pub fn fill_square(src: &RgbaImage, size: u32) -> RgbaImage {
    let (w, h) = src.dimensions();
    let side = w.min(h);
    let (left, top) = ((w - side) / 2, (h - side) / 2);
    let Ok(s) = fr::images::ImageRef::new(w, h, src.as_raw(), fr::PixelType::U8x4) else { return src.clone() };
    let mut d = fr::images::Image::new(size, size, fr::PixelType::U8x4);
    let opts = fr::ResizeOptions::new()
        .resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Bilinear))
        .crop(left as f64, top as f64, side as f64, side as f64);
    if fr::Resizer::new().resize(&s, &mut d, &opts).is_err() {
        return src.clone();
    }
    RgbaImage::from_raw(size, size, d.into_vec()).unwrap_or_else(|| src.clone())
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("RustPhotos").join("thumbs")
}

fn key(path: &Path, size: u32) -> Option<String> {
    let m = std::fs::metadata(path).ok()?;
    let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    let s = format!("{}|{}|{}|{}|v3", path.to_string_lossy(), m.len(), mtime, size);
    Some(format!("{:016x}", xxhash_rust::xxh3::xxh3_64(s.as_bytes())))
}

/// Returns the thumbnail of a picture, creating (and caching) it on first use.
/// The EXIF orientation is applied, so phone photos are never rotated wrongly.
pub fn load_thumb(dir: &Path, path: &Path, size: u32) -> Result<RgbaImage, String> {
    let k = key(path, size).ok_or("no metadata")?;
    let out = dir.join(format!("{k}.jpg"));
    if let Ok(img) = image::open(&out) {
        return Ok(img.to_rgba8());
    }
    // JPEG (most photos): decode at 1/2, 1/4 or 1/8 size straight away. Anything else: full decode.
    let rgb: RgbImage = match if decode::is_jpeg(path) { decode::jpeg_scaled(path, (size * 2) as u16).ok() } else { None } {
        Some(small) => fill_square_rgb(&small, size),
        None => {
            let img = decode::open_upright(path)?;
            if img.color().has_alpha() {
                // Transparent pictures get a light backdrop in the cached JPEG.
                let t = fill_square(&img.to_rgba8(), size);
                let mut rgb = RgbImage::new(size, size);
                for (d, s) in rgb.pixels_mut().zip(t.pixels()) {
                    let a = s[3] as u32;
                    let mix = |c: u8| ((c as u32 * a + 230 * (255 - a)) / 255) as u8;
                    *d = image::Rgb([mix(s[0]), mix(s[1]), mix(s[2])]);
                }
                rgb
            } else {
                fill_square_rgb(&img.into_rgb8(), size)
            }
        }
    };
    let _ = std::fs::create_dir_all(dir);
    let tmp = out.with_extension("tmp");
    if rgb.save_with_format(&tmp, image::ImageFormat::Jpeg).is_ok() {
        let _ = std::fs::rename(&tmp, &out);
    }
    Ok(image::DynamicImage::ImageRgb8(rgb).to_rgba8())
}

/// Removes cached thumbnails that have not been refreshed for 45 days.
pub fn prune(dir: &Path) {
    let Some(cutoff) = SystemTime::now().checked_sub(Duration::from_secs(60 * 60 * 24 * 45)) else { return };
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                if t < cutoff {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}
