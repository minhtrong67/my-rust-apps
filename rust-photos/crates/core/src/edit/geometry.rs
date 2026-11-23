//! Rotate by quarter turns, flip and straighten.
use super::state::EditState;
use crate::thumbs::resize_to;
use fast_image_resize::FilterType;
use image::{imageops, Rgba, RgbaImage};
use rayon::prelude::*;

fn sample(img: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let g = |xi: i32, yi: i32| img.get_pixel(xi.clamp(0, w - 1) as u32, yi.clamp(0, h - 1) as u32).0;
    let (xi, yi) = (x0 as i32, y0 as i32);
    let (a, b, c, d) = (g(xi, yi), g(xi + 1, yi), g(xi, yi + 1), g(xi + 1, yi + 1));
    let mut out = [0u8; 4];
    for i in 0..4 {
        let top = a[i] as f32 * (1.0 - fx) + b[i] as f32 * fx;
        let bot = c[i] as f32 * (1.0 - fx) + d[i] as f32 * fx;
        out[i] = (top * (1.0 - fy) + bot * fy + 0.5) as u8;
    }
    Rgba(out)
}

/// The picture after rotate, flip and straighten, scaled so its longest side is at most `max_dim`.
pub fn geometry(src: &RgbaImage, st: &EditState, max_dim: u32) -> RgbaImage {
    let (sw, sh) = src.dimensions();
    let swap = st.rot % 2 == 1;
    let (bw, bh) = if swap { (sh, sw) } else { (sw, sh) };
    let k = (max_dim as f64 / bw.max(bh) as f64).min(1.0);
    let scaled;
    let base: &RgbaImage = if k < 1.0 {
        scaled = resize_to(src, ((sw as f64 * k).round() as u32).max(1), ((sh as f64 * k).round() as u32).max(1), FilterType::Lanczos3);
        &scaled
    } else {
        src
    };
    let mut img = match st.rot % 4 {
        1 => imageops::rotate90(base),
        2 => imageops::rotate180(base),
        3 => imageops::rotate270(base),
        _ => base.clone(),
    };
    if st.flip_h { imageops::flip_horizontal_in_place(&mut img); }
    if st.flip_v { imageops::flip_vertical_in_place(&mut img); }
    if st.angle.abs() < 0.001 {
        return img;
    }
    let (w, h) = img.dimensions();
    let th = st.angle.to_radians();
    let (sn, cs) = th.sin_cos();
    let ratio = (w as f32 / h as f32).max(h as f32 / w as f32);
    let cover = th.cos().abs() + th.sin().abs() * ratio;
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let mut out = RgbaImage::new(w, h);
    out.par_chunks_mut(w as usize * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w as usize {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let (sx, sy) = ((dx * cs + dy * sn) / cover + cx - 0.5, (-dx * sn + dy * cs) / cover + cy - 0.5);
            row[x * 4..x * 4 + 4].copy_from_slice(&sample(&img, sx, sy).0);
        }
    });
    out
}
