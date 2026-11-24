//! Decoding pictures from disk: raster formats through `image`, SVG through `resvg`.
use crate::exif_util;
use image::{DynamicImage, RgbImage, RgbaImage};
use std::path::Path;

pub fn is_svg(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("svg")).unwrap_or(false)
}

/// Rasterises an SVG so its longest side is `target` pixels (default 2048).
pub fn render_svg(path: &Path, target: u32) -> Result<RgbaImage, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).map_err(|e| e.to_string())?;
    let size = tree.size();
    let k = target as f32 / size.width().max(size.height()).max(1.0);
    let (w, h) = (((size.width() * k).round() as u32).max(1), ((size.height() * k).round() as u32).max(1));
    let mut pm = tiny_skia::Pixmap::new(w, h).ok_or("svg too large")?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
    let mut out = RgbaImage::new(w, h);
    for (dst, src) in out.pixels_mut().zip(pm.pixels()) {
        let c = src.demultiply();
        *dst = image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Ok(out)
}

/// Opens a picture and applies its EXIF orientation, so the result always looks upright.
pub fn open_upright(path: &Path) -> Result<DynamicImage, String> {
    if is_svg(path) {
        return render_svg(path, 2048).map(DynamicImage::ImageRgba8);
    }
    let img = image::open(path).map_err(|e| e.to_string())?;
    Ok(exif_util::apply_orientation(img, exif_util::orientation(path)))
}

/// Same as `open_upright`, but the long side is limited so huge panoramas stay GPU friendly.
pub fn open_for_view(path: &Path, max_side: u32) -> Result<RgbaImage, String> {
    let img = open_upright(path)?;
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= max_side {
        return Ok(img.to_rgba8());
    }
    Ok(crate::thumbs::resize_rgba(&img.to_rgba8(), max_side))
}

pub fn is_jpeg(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| matches!(e.to_lowercase().as_str(), "jpg" | "jpeg" | "jfif")).unwrap_or(false)
}

/// Decodes a JPEG already reduced by the decoder (1/2, 1/4 or 1/8 size, done in the DCT domain) so both
/// sides stay at least `min_side` pixels. For a thumbnail this skips almost all of the work and memory of a
/// full decode. The result is upright. Errors for formats it does not handle (CMYK, 16 bit): use `open_upright`.
pub fn jpeg_scaled(path: &Path, min_side: u16) -> Result<RgbImage, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut d = jpeg_decoder::Decoder::new(std::io::BufReader::new(f));
    d.read_info().map_err(|e| e.to_string())?;
    d.scale(min_side, min_side).map_err(|e| e.to_string())?;
    let px = d.decode().map_err(|e| e.to_string())?;
    let info = d.info().ok_or("no jpeg info")?;
    let (w, h) = (info.width as u32, info.height as u32);
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => RgbImage::from_raw(w, h, px).ok_or("bad size")?,
        jpeg_decoder::PixelFormat::L8 => RgbImage::from_fn(w, h, |x, y| { let v = px[(y * w + x) as usize]; image::Rgb([v, v, v]) }),
        _ => return Err("unsupported jpeg".into()),
    };
    Ok(exif_util::apply_orientation(DynamicImage::ImageRgb8(rgb), exif_util::orientation(path)).into_rgb8())
}
