//! Drawings on top of the picture: pen, highlighter, shapes, text and the blur / pixelate boxes.
use super::state::{BlurMode, Item, ShapeKind};
use crate::thumbs::resize_to;
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use fast_image_resize::FilterType;
use image::RgbaImage;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, PremultipliedColorU8, Rect, Stroke, Transform};

type Key = (bool, bool);
static FONTS: OnceLock<Mutex<(fontdb::Database, HashMap<Key, Option<FontArc>>)>> = OnceLock::new();

/// A system sans-serif font (Segoe UI on Windows) in the wanted weight and style.
fn font(bold: bool, italic: bool) -> Option<FontArc> {
    let m = FONTS.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Mutex::new((db, HashMap::new()))
    });
    let mut g = m.lock().ok()?;
    if let Some(f) = g.1.get(&(bold, italic)) {
        return f.clone();
    }
    let fams = [
        fontdb::Family::Name("Segoe UI"),
        fontdb::Family::Name("Roboto"),
        fontdb::Family::Name("Noto Sans"),
        fontdb::Family::Name("DejaVu Sans"),
        fontdb::Family::Name("Arial"),
        fontdb::Family::SansSerif,
    ];
    let q = fontdb::Query {
        families: &fams,
        weight: if bold { fontdb::Weight::BOLD } else { fontdb::Weight::MEDIUM },
        stretch: fontdb::Stretch::Normal,
        style: if italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
    };
    let found = g.0.query(&q).and_then(|id| g.0.with_face_data(id, |d, _| FontArc::try_from_vec(d.to_vec()).ok()).flatten());
    g.1.insert((bold, italic), found.clone());
    found
}

fn paint(c: [u8; 4], alpha_mul: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], (c[3] as f32 * alpha_mul).round().clamp(0.0, 255.0) as u8);
    p.anti_alias = true;
    p
}

fn to_pixmap(img: &RgbaImage) -> Pixmap {
    let mut pm = Pixmap::new(img.width(), img.height()).expect("pixmap");
    for (d, s) in pm.pixels_mut().iter_mut().zip(img.pixels()) {
        let a = s[3] as u32;
        let m = |c: u8| ((c as u32 * a + 127) / 255) as u8;
        *d = PremultipliedColorU8::from_rgba(m(s[0]), m(s[1]), m(s[2]), s[3]).unwrap_or(PremultipliedColorU8::TRANSPARENT);
    }
    pm
}

fn from_pixmap(pm: &Pixmap, img: &mut RgbaImage) {
    for (d, s) in img.pixels_mut().zip(pm.pixels()) {
        let c = s.demultiply();
        *d = image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
}

/// Measures a text item: (width, height) of its box in pixels, and the line height.
pub fn text_box(text: &str, size_px: f32, bold: bool, italic: bool) -> (f32, f32, f32) {
    let lines: Vec<&str> = text.split('\n').collect();
    let lh = size_px * 1.22;
    let mut w = 0f32;
    if let Some(f) = font(bold, italic) {
        let sc = f.as_scaled(PxScale::from(size_px));
        for l in &lines {
            let mut x = 0.0;
            for ch in if l.is_empty() { " ".chars().collect::<Vec<_>>() } else { l.chars().collect() } {
                x += sc.h_advance(sc.glyph_id(ch));
            }
            w = w.max(x);
        }
    } else {
        w = lines.iter().map(|l| l.chars().count() as f32 * size_px * 0.55).fold(0.0, f32::max);
    }
    (w + size_px * 0.5, lines.len() as f32 * lh + size_px * 0.3, lh)
}

fn glyph_mask(text: &str, size_px: f32, bold: bool, italic: bool, bw: f32, bh: f32, lh: f32, w: usize, h: usize, ox: f32, oy: f32) -> Vec<f32> {
    let mut mask = vec![0f32; w * h];
    let Some(f) = font(bold, italic) else { return mask };
    let sc = f.as_scaled(PxScale::from(size_px));
    let lines: Vec<&str> = text.split('\n').collect();
    let n = lines.len() as f32;
    for (i, line) in lines.iter().enumerate() {
        let lw: f32 = line.chars().map(|c| sc.h_advance(sc.glyph_id(c))).sum();
        let mut x = ox + bw / 2.0 - lw / 2.0;
        let baseline = oy + bh / 2.0 + (i as f32 - (n - 1.0) / 2.0) * lh + (sc.ascent() + sc.descent()) / 2.0;
        for ch in line.chars() {
            let gid = sc.glyph_id(ch);
            let g = gid.with_scale_and_position(PxScale::from(size_px), ab_glyph::point(x, baseline));
            if let Some(og) = f.outline_glyph(g) {
                let b = og.px_bounds();
                og.draw(|gx, gy, c| {
                    let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                        let idx = py as usize * w + px as usize;
                        mask[idx] = (mask[idx] + c).min(1.0);
                    }
                });
            }
            x += sc.h_advance(gid);
        }
    }
    mask
}

fn blit_mask(pm: &mut Pixmap, mask: &[f32], color: [u8; 4], alpha: f32) {
    for (d, m) in pm.pixels_mut().iter_mut().zip(mask) {
        if *m <= 0.0 { continue; }
        let a = (m * alpha * color[3] as f32 / 255.0).clamp(0.0, 1.0);
        let pr = |c: u8| (c as f32 * a) as u8;
        let (sr, sg, sb, sa) = (pr(color[0]), pr(color[1]), pr(color[2]), (a * 255.0) as u8);
        let inv = 255 - sa as u32;
        let over = |s: u8, d: u8| (s as u32 + (d as u32 * inv + 127) / 255).min(255) as u8;
        *d = PremultipliedColorU8::from_rgba(over(sr, d.red()), over(sg, d.green()), over(sb, d.blue()), over(sa, d.alpha())).unwrap_or(*d);
    }
}

fn draw_text(pm: &mut Pixmap, it: &Item) {
    let Item::Text { text, x, y, size, rot, color, bold, italic, bg, .. } = it else { return };
    let (cw, ch) = (pm.width() as f32, pm.height() as f32);
    let px = size * cw;
    if px < 1.0 { return; }
    let (bw, bh, lh) = text_box(text, px, *bold, *italic);
    let margin = (px * 0.5).ceil();
    let (tw, th) = ((bw + margin * 2.0).ceil() as u32, (bh + margin * 2.0).ceil() as u32);
    let Some(mut tmp) = Pixmap::new(tw.max(1), th.max(1)) else { return };
    let (w, h) = (tw as usize, th as usize);
    if *bg {
        let mut pb = PathBuilder::new();
        let r = px * 0.35;
        let (x0, y0, x1, y1) = (margin, margin, margin + bw, margin + bh);
        pb.move_to(x0 + r, y0);
        pb.line_to(x1 - r, y0); pb.quad_to(x1, y0, x1, y0 + r);
        pb.line_to(x1, y1 - r); pb.quad_to(x1, y1, x1 - r, y1);
        pb.line_to(x0 + r, y1); pb.quad_to(x0, y1, x0, y1 - r);
        pb.line_to(x0, y0 + r); pb.quad_to(x0, y0, x0 + r, y0);
        pb.close();
        if let Some(p) = pb.finish() { tmp.fill_path(&p, &paint([0, 0, 0, 140], 1.0), FillRule::Winding, Transform::identity(), None); }
    } else {
        let mut sh = glyph_mask(text, px, *bold, *italic, bw, bh, lh, w, h, margin, margin + px * 0.04);
        let mut mimg = RgbaImage::new(tw, th);
        for (p, m) in mimg.pixels_mut().zip(&sh) { p[3] = (m * 255.0) as u8; }
        super::color::blur(&mut mimg, (px * 0.06).max(0.6));
        for (m, p) in sh.iter_mut().zip(mimg.pixels()) { *m = p[3] as f32 / 255.0; }
        blit_mask(&mut tmp, &sh, [0, 0, 0, 255], 0.45);
    }
    let mut mask = glyph_mask(text, px, *bold, *italic, bw, bh, lh, w, h, margin, margin);
    if *italic && font(*bold, true).is_none() {
        // No italic face installed: shear the glyphs a little.
        let src = mask.clone();
        for yy in 0..h {
            let shift = ((h as f32 / 2.0 - yy as f32) * 0.2) as i32;
            for xx in 0..w {
                let sx = xx as i32 - shift;
                mask[yy * w + xx] = if sx >= 0 && (sx as usize) < w { src[yy * w + sx as usize] } else { 0.0 };
            }
        }
    }
    blit_mask(&mut tmp, &mask, *color, 1.0);
    let tf = Transform::from_translate(x * cw, y * ch).pre_rotate(rot.to_degrees()).pre_translate(-(tw as f32) / 2.0, -(th as f32) / 2.0);
    pm.draw_pixmap(0, 0, tmp.as_ref(), &PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() }, tf, None);
}

fn draw_blur_box(pm: &mut Pixmap, it: &Item) {
    let Item::Blur { x, y, w, h, mode, .. } = it else { return };
    let (cw, ch) = (pm.width() as f32, pm.height() as f32);
    let k = cw / 1000.0;
    let x0 = (x * cw).round().max(0.0) as u32;
    let y0 = (y * ch).round().max(0.0) as u32;
    let rw = ((w * cw).round() as u32).min(pm.width().saturating_sub(x0));
    let rh = ((h * ch).round() as u32).min(pm.height().saturating_sub(y0));
    if rw < 2 || rh < 2 { return; }
    let mut region = RgbaImage::new(rw, rh);
    for yy in 0..rh {
        for xx in 0..rw {
            let c = pm.pixel(x0 + xx, y0 + yy).map(|p| p.demultiply()).unwrap();
            region.put_pixel(xx, yy, image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]));
        }
    }
    let region = match mode {
        BlurMode::Pixelate => {
            let step = (6.0 * k).max(rw.min(rh) as f32 / 12.0);
            let small = resize_to(&region, ((rw as f32 / step) as u32).max(1), ((rh as f32 / step) as u32).max(1), FilterType::Box);
            image::imageops::resize(&small, rw, rh, image::imageops::FilterType::Nearest)
        }
        BlurMode::Blur => {
            let mut r = region;
            super::color::blur(&mut r, (14.0 * k).max(4.0));
            r
        }
    };
    let pw = pm.width();
    let data = pm.pixels_mut();
    for yy in 0..rh {
        for xx in 0..rw {
            let c = region.get_pixel(xx, yy);
            let a = c[3] as u32;
            let m = |v: u8| ((v as u32 * a + 127) / 255) as u8;
            data[((y0 + yy) * pw + x0 + xx) as usize] = PremultipliedColorU8::from_rgba(m(c[0]), m(c[1]), m(c[2]), c[3]).unwrap_or(PremultipliedColorU8::TRANSPARENT);
        }
    }
}

/// Draws all items in order. Item sizes are relative to the picture width, so any output size looks the same.
pub fn draw_items(img: &mut RgbaImage, items: &[Item], skip: Option<u64>) {
    if items.is_empty() {
        return;
    }
    let mut pm = to_pixmap(img);
    let (cw, ch) = (pm.width() as f32, pm.height() as f32);
    let k = cw / 1000.0;
    for it in items {
        if Some(it.id()) == skip { continue; }
        match it {
            Item::Stroke { pts, color, width, highlighter, .. } => {
                if pts.is_empty() { continue; }
                let mut pb = PathBuilder::new();
                pb.move_to(pts[0][0] * cw, pts[0][1] * ch);
                if pts.len() == 1 { pb.line_to(pts[0][0] * cw + 0.01, pts[0][1] * ch); }
                for i in 1..pts.len().saturating_sub(1) {
                    let (mx, my) = ((pts[i][0] + pts[i + 1][0]) / 2.0 * cw, (pts[i][1] + pts[i + 1][1]) / 2.0 * ch);
                    pb.quad_to(pts[i][0] * cw, pts[i][1] * ch, mx, my);
                }
                if pts.len() > 1 { let l = pts[pts.len() - 1]; pb.line_to(l[0] * cw, l[1] * ch); }
                if let Some(p) = pb.finish() {
                    let st = Stroke { width: width * k * if *highlighter { 3.0 } else { 1.0 }, line_cap: if *highlighter { LineCap::Butt } else { LineCap::Round }, line_join: LineJoin::Round, ..Default::default() };
                    pm.stroke_path(&p, &paint(*color, if *highlighter { 0.38 } else { 1.0 }), &st, Transform::identity(), None);
                }
            }
            Item::Shape { shape, a, b, color, width, fill, .. } => {
                let (ax, ay, bx, by) = (a[0] * cw, a[1] * ch, b[0] * cw, b[1] * ch);
                let p = paint(*color, 1.0);
                let st = Stroke { width: width * k, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
                match shape {
                    ShapeKind::Rect | ShapeKind::Oval => {
                        let Some(r) = Rect::from_ltrb(ax.min(bx), ay.min(by), ax.max(bx).max(ax.min(bx) + 0.01), ay.max(by).max(ay.min(by) + 0.01)) else { continue };
                        let path = if matches!(shape, ShapeKind::Rect) { Some(PathBuilder::from_rect(r)) } else { PathBuilder::from_oval(r) };
                        if let Some(path) = path {
                            if *fill { pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None); } else { pm.stroke_path(&path, &p, &st, Transform::identity(), None); }
                        }
                    }
                    ShapeKind::Line | ShapeKind::Arrow => {
                        let mut pb = PathBuilder::new();
                        pb.move_to(ax, ay);
                        pb.line_to(bx, by);
                        if let Some(path) = pb.finish() { pm.stroke_path(&path, &p, &st, Transform::identity(), None); }
                        if matches!(shape, ShapeKind::Arrow) {
                            let ang = (by - ay).atan2(bx - ax);
                            let hl = (14.0 * k).max(width * k * 4.0);
                            let mut hb = PathBuilder::new();
                            hb.move_to(bx, by);
                            hb.line_to(bx - hl * (ang - 0.45).cos(), by - hl * (ang - 0.45).sin());
                            hb.line_to(bx - hl * (ang + 0.45).cos(), by - hl * (ang + 0.45).sin());
                            hb.close();
                            if let Some(path) = hb.finish() { pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None); }
                        }
                    }
                }
            }
            Item::Text { .. } => draw_text(&mut pm, it),
            // Colour emoji need a COLR renderer; not ported yet.
            Item::Emoji { .. } => {}
            Item::Blur { .. } => draw_blur_box(&mut pm, it),
        }
    }
    from_pixmap(&pm, img);
}
