//! Photo editing engine (replaces the canvas based `editorEngine.ts`).
pub mod color;
pub mod draw;
pub mod geometry;
pub mod state;

pub use state::*;

use crate::{exif_util, thumbs::resize_to};
use fast_image_resize::FilterType;
use image::RgbaImage;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Format {
    Png,
    Jpeg,
    Webp,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self { Format::Png => "png", Format::Jpeg => "jpg", Format::Webp => "webp" }
    }
    pub fn from_ext(e: &str) -> Format {
        match e.to_lowercase().as_str() { "jpg" | "jpeg" | "jfif" => Format::Jpeg, "webp" => Format::Webp, _ => Format::Png }
    }
}

#[derive(Clone, Debug)]
pub struct ExportOpts {
    pub format: Format,
    /// 1..=100, used by JPEG.
    pub quality: u8,
    /// Output size; `None` keeps the edited size.
    pub size: Option<(u32, u32)>,
}

#[derive(Default, Clone)]
pub struct RenderOpts {
    /// Show the whole picture, ignoring the crop (used while the crop tool is open).
    pub full: bool,
    /// Skip every edit and show the original (hold to compare).
    pub plain: bool,
    /// An item that is still being drawn.
    pub draft: Option<Item>,
    /// An item that is being dragged (drawn by the UI instead).
    pub hide_id: Option<u64>,
}

pub struct Engine {
    src: RgbaImage,
    geo_key: String,
    geo: Option<RgbaImage>,
    adj_key: String,
    adj: Option<RgbaImage>,
}

impl Engine {
    pub fn new(src: RgbaImage) -> Engine {
        Engine { src, geo_key: String::new(), geo: None, adj_key: String::new(), adj: None }
    }

    pub fn width(&self) -> u32 { self.src.width() }
    pub fn height(&self) -> u32 { self.src.height() }

    fn geometry(&mut self, st: &EditState, max_dim: u32) -> &RgbaImage {
        let key = format!("{}|{}|{}|{}|{}", st.rot, st.flip_h, st.flip_v, st.angle, max_dim);
        if key != self.geo_key || self.geo.is_none() {
            self.geo = Some(geometry::geometry(&self.src, st, max_dim));
            self.geo_key = key;
            self.adj = None;
        }
        self.geo.as_ref().unwrap()
    }

    /// Geometry, crop, colour adjustments and the preset filter (no drawings). Cached while only the drawings change.
    fn adjusted(&mut self, st: &EditState, max_dim: u32, use_crop: bool) -> RgbaImage {
        let key = format!("{}|{}|{}|{}|{}|{:?}|{:?}|{}|{}", st.rot, st.flip_h, st.flip_v, st.angle, max_dim, if use_crop { Some(st.crop) } else { None }, st.adj, st.filter, st.filter_amt);
        self.geometry(st, max_dim);
        if key == self.adj_key {
            if let Some(a) = &self.adj {
                return a.clone();
            }
        }
        let base = self.geo.as_ref().unwrap();
        let r = if use_crop { st.crop } else { Crop::default() };
        let (bw, bh) = base.dimensions();
        let (cx, cy) = ((r.x * bw as f32).round() as u32, (r.y * bh as f32).round() as u32);
        let (cw, ch) = (((r.w * bw as f32).round() as u32).clamp(1, bw - cx.min(bw - 1)), ((r.h * bh as f32).round() as u32).clamp(1, bh - cy.min(bh - 1)));
        let mut out = image::imageops::crop_imm(base, cx.min(bw - 1), cy.min(bh - 1), cw, ch).to_image();
        let scale = out.width() as f32 / 1000.0;
        color::apply_ops(&mut out, &color::adj_ops(&st.adj));
        color::blur(&mut out, st.adj.blur / 100.0 * 14.0 * scale);
        if let Some(p) = color::preset(&st.filter) {
            let mut b = out.clone();
            color::apply_ops(&mut b, &p.ops);
            color::blur(&mut b, p.blur_px);
            if st.filter_amt >= 100.0 { out = b; } else { color::mix(&mut out, &b, st.filter_amt / 100.0); }
        }
        color::pixel_ops(&mut out, &st.adj);
        self.adj_key = key;
        self.adj = Some(out.clone());
        out
    }

    /// Renders the picture for the screen.
    pub fn preview(&mut self, st: &EditState, max_dim: u32, o: &RenderOpts) -> RgbaImage {
        if o.plain {
            return crate::thumbs::resize_rgba(&self.src, max_dim);
        }
        let mut out = self.adjusted(st, max_dim, !o.full);
        let mut items: Vec<Item> = st.items.clone();
        if let Some(d) = &o.draft { items.push(d.clone()); }
        draw::draw_items(&mut out, &items, o.hide_id);
        out
    }

    /// Full resolution result, optionally resized.
    pub fn export_image(&mut self, st: &EditState, size: Option<(u32, u32)>) -> RgbaImage {
        let mut out = self.adjusted(st, u32::MAX, true);
        draw::draw_items(&mut out, &st.items, None);
        match size {
            Some((w, h)) if (w, h) != out.dimensions() && w > 0 && h > 0 => resize_to(&out, w, h, FilterType::Lanczos3),
            _ => out,
        }
    }

    /// Final size after rotate and crop, without rendering anything.
    pub fn crop_size(&self, st: &EditState) -> (u32, u32) {
        let (sw, sh) = if st.rot % 2 == 1 { (self.src.height(), self.src.width()) } else { (self.src.width(), self.src.height()) };
        (((sw as f32 * st.crop.w).round() as u32).max(1), ((sh as f32 * st.crop.h).round() as u32).max(1))
    }

    /// Suggested adjustments from the histogram (stretches contrast, lifts shadows, adds a little colour).
    pub fn auto_enhance(&mut self, st: &EditState) -> Adj {
        let mut s = st.clone();
        s.angle = 0.0;
        let c = self.geometry(&s, 256).clone();
        let mut hist = [0u32; 256];
        let (mut n, mut sat) = (0u64, 0u64);
        for p in c.pixels() {
            let l = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).round() as usize;
            hist[l.min(255)] += 1;
            n += 1;
            sat += (p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2])) as u64;
        }
        let (mut acc, mut p1, mut p99) = (0u64, 0usize, 255usize);
        for i in 0..256 { acc += hist[i] as u64; if acc as f64 >= n as f64 * 0.01 { p1 = i; break; } }
        acc = 0;
        for i in (0..256).rev() { acc += hist[i] as u64; if acc as f64 >= n as f64 * 0.01 { p99 = i; break; } }
        let range = (p99 as f32 - p1 as f32).max(30.0);
        let mid = (p1 + p99) as f32 / 2.0;
        let cl = |v: f32, m: f32| v.clamp(-m, m).round();
        Adj {
            contrast: cl((255.0 / range - 1.0) * 70.0, 40.0),
            brightness: cl((128.0 - mid) / 2.55 * 0.5, 25.0),
            saturation: if (sat as f64 / n.max(1) as f64) < 40.0 { 18.0 } else { 8.0 },
            shadows: 12.0,
            highlights: -8.0,
            ..Adj::default()
        }
    }

    /// Small preview of a filter applied to the picture (for the filter strip).
    pub fn filter_preview(&mut self, filter: &str, size: u32) -> RgbaImage {
        let mut st = EditState::default();
        st.filter = filter.to_string();
        let mut img = geometry::geometry(&self.src, &EditState::default(), size);
        if let Some(p) = color::preset(filter) {
            color::apply_ops(&mut img, &p.ops);
            color::blur(&mut img, p.blur_px);
        }
        img
    }
}

/// Encodes for saving. JPEG is flattened on white. WebP is lossless (the encoder has no lossy mode).
pub fn encode(img: &RgbaImage, o: &ExportOpts) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    match o.format {
        Format::Png => img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).map_err(|e| e.to_string())?,
        Format::Webp => img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::WebP).map_err(|e| e.to_string())?,
        Format::Jpeg => {
            let mut rgb = image::RgbImage::new(img.width(), img.height());
            for (d, s) in rgb.pixels_mut().zip(img.pixels()) {
                let a = s[3] as u32;
                let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
                *d = image::Rgb([mix(s[0]), mix(s[1]), mix(s[2])]);
            }
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, o.quality.clamp(1, 100));
            image::DynamicImage::ImageRgb8(rgb).write_with_encoder(enc).map_err(|e| e.to_string())?;
        }
    }
    Ok(buf)
}

/// Renders, encodes and writes the edit. With `exif_from`, the camera data of that JPEG is kept.
pub fn save(engine: &mut Engine, st: &EditState, o: &ExportOpts, path: &std::path::Path, exif_from: Option<&std::path::Path>) -> Result<(), String> {
    let img = engine.export_image(st, o.size);
    let mut bytes = encode(&img, o)?;
    if o.format == Format::Jpeg {
        if let Some(src) = exif_from.and_then(|p| std::fs::read(p).ok()) {
            bytes = exif_util::carry_exif(&src, bytes);
        }
    }
    crate::files::write_atomic(path, &bytes)
}

/// Luminance and RGB histograms (256 bins each) of a picture.
pub fn histogram(img: &RgbaImage) -> [[u32; 256]; 4] {
    let mut h = [[0u32; 256]; 4];
    for p in img.pixels() {
        h[0][p[0] as usize] += 1;
        h[1][p[1] as usize] += 1;
        h[2][p[2] as usize] += 1;
        h[3][(0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) as usize] += 1;
    }
    h
}
