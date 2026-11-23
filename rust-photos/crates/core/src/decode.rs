//! Decoding pictures from disk: raster formats through `image`, SVG through `resvg`.
use crate::exif_util;
use image::{DynamicImage, RgbaImage};
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
