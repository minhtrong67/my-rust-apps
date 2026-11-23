//! Colour work: CSS-style filter functions, the pixel level adjustments and a fast gaussian-like blur.
use super::state::Adj;
use image::RgbaImage;
use rayon::prelude::*;

#[derive(Clone, Copy, Debug)]
pub enum Op {
    Brightness(f32),
    Contrast(f32),
    Saturate(f32),
    Hue(f32),
    Sepia(f32),
    Grayscale(f32),
}

pub struct FilterPreset {
    pub ops: Vec<Op>,
    pub blur_px: f32,
}

/// The twelve preset looks, written with the same numbers as the CSS filters of the old canvas engine.
pub fn preset(id: &str) -> Option<FilterPreset> {
    use Op::*;
    let (ops, blur_px) = match id {
        "vivid" => (vec![Saturate(1.45), Contrast(1.1), Brightness(1.03)], 0.0),
        "warm" => (vec![Sepia(0.25), Saturate(1.25), Hue(-8.0), Brightness(1.05)], 0.0),
        "cool" => (vec![Hue(14.0), Saturate(1.1), Brightness(1.03), Contrast(1.04)], 0.0),
        "mono" => (vec![Grayscale(1.0), Contrast(1.05)], 0.0),
        "noir" => (vec![Grayscale(1.0), Contrast(1.45), Brightness(0.88)], 0.0),
        "sepia" => (vec![Sepia(0.9), Contrast(1.05)], 0.0),
        "fade" => (vec![Contrast(0.84), Brightness(1.12), Saturate(0.78)], 0.0),
        "dramatic" => (vec![Contrast(1.38), Saturate(1.18), Brightness(0.92)], 0.0),
        "sunset" => (vec![Sepia(0.32), Saturate(1.6), Hue(-16.0), Contrast(1.05)], 0.0),
        "dream" => (vec![Brightness(1.1), Saturate(1.2), Contrast(0.92)], 0.6),
        "film" => (vec![Sepia(0.18), Contrast(1.12), Saturate(0.92), Brightness(1.03)], 0.0),
        _ => return None,
    };
    Some(FilterPreset { ops, blur_px })
}

/// Brightness / contrast / saturation / hue from the sliders, in the CSS order.
pub fn adj_ops(a: &Adj) -> Vec<Op> {
    let mut v = Vec::new();
    if a.brightness != 0.0 { v.push(Op::Brightness(1.0 + a.brightness / 100.0)); }
    if a.contrast != 0.0 { v.push(Op::Contrast(1.0 + a.contrast / 100.0)); }
    if a.saturation != 0.0 { v.push(Op::Saturate((1.0 + a.saturation / 100.0).max(0.0))); }
    if a.hue != 0.0 { v.push(Op::Hue(a.hue)); }
    v
}

type M = [[f32; 3]; 3];

fn matrix(op: Op) -> Option<M> {
    Some(match op {
        Op::Saturate(s) => [
            [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
            [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
            [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
        ],
        Op::Hue(deg) => {
            let (sn, cs) = deg.to_radians().sin_cos();
            [
                [0.213 + cs * 0.787 - sn * 0.213, 0.715 - cs * 0.715 - sn * 0.715, 0.072 - cs * 0.072 + sn * 0.928],
                [0.213 - cs * 0.213 + sn * 0.143, 0.715 + cs * 0.285 + sn * 0.140, 0.072 - cs * 0.072 - sn * 0.283],
                [0.213 - cs * 0.213 - sn * 0.787, 0.715 - cs * 0.715 + sn * 0.715, 0.072 + cs * 0.928 + sn * 0.072],
            ]
        }
        Op::Grayscale(a) => {
            let k = 1.0 - a.clamp(0.0, 1.0);
            [
                [0.2126 + 0.7874 * k, 0.7152 - 0.7152 * k, 0.0722 - 0.0722 * k],
                [0.2126 - 0.2126 * k, 0.7152 + 0.2848 * k, 0.0722 - 0.0722 * k],
                [0.2126 - 0.2126 * k, 0.7152 - 0.7152 * k, 0.0722 + 0.9278 * k],
            ]
        }
        Op::Sepia(a) => {
            let k = 1.0 - a.clamp(0.0, 1.0);
            [
                [0.393 + 0.607 * k, 0.769 - 0.769 * k, 0.189 - 0.189 * k],
                [0.349 - 0.349 * k, 0.686 + 0.314 * k, 0.168 - 0.168 * k],
                [0.272 - 0.272 * k, 0.534 - 0.534 * k, 0.131 + 0.869 * k],
            ]
        }
        _ => return None,
    })
}

#[inline]
fn apply_op(c: [f32; 3], op: Op, m: &Option<M>) -> [f32; 3] {
    let out = match (op, m) {
        (Op::Brightness(b), _) => [c[0] * b, c[1] * b, c[2] * b],
        (Op::Contrast(k), _) => [(c[0] - 0.5) * k + 0.5, (c[1] - 0.5) * k + 0.5, (c[2] - 0.5) * k + 0.5],
        (_, Some(m)) => [
            m[0][0] * c[0] + m[0][1] * c[1] + m[0][2] * c[2],
            m[1][0] * c[0] + m[1][1] * c[1] + m[1][2] * c[2],
            m[2][0] * c[0] + m[2][1] * c[1] + m[2][2] * c[2],
        ],
        _ => c,
    };
    [out[0].clamp(0.0, 1.0), out[1].clamp(0.0, 1.0), out[2].clamp(0.0, 1.0)]
}

/// Applies a chain of filter functions to every pixel (each step is clamped, like CSS filters).
pub fn apply_ops(img: &mut RgbaImage, ops: &[Op]) {
    if ops.is_empty() {
        return;
    }
    let mats: Vec<Option<M>> = ops.iter().map(|o| matrix(*o)).collect();
    let (w, _) = img.dimensions();
    img.par_chunks_mut(w as usize * 4).for_each(|row| {
        for px in row.chunks_exact_mut(4) {
            let mut c = [px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0];
            for (op, m) in ops.iter().zip(&mats) {
                c = apply_op(c, *op, m);
            }
            px[0] = (c[0] * 255.0 + 0.5) as u8;
            px[1] = (c[1] * 255.0 + 0.5) as u8;
            px[2] = (c[2] * 255.0 + 0.5) as u8;
        }
    });
}

/// Linear blend of two equally sized images: `a + (b - a) * t`.
pub fn mix(a: &mut RgbaImage, b: &RgbaImage, t: f32) {
    let t = t.clamp(0.0, 1.0);
    a.par_chunks_mut(4096).zip(b.par_chunks(4096)).for_each(|(x, y)| {
        for (p, q) in x.iter_mut().zip(y) {
            *p = (*p as f32 + (*q as f32 - *p as f32) * t + 0.5) as u8;
        }
    });
}

/// Warmth, tint, shadows, highlights, sharpness and vignette (pixel level work, same maths as before).
pub fn pixel_ops(img: &mut RgbaImage, a: &Adj) {
    if a.warmth == 0.0 && a.tint == 0.0 && a.highlights == 0.0 && a.shadows == 0.0 && a.sharpness == 0.0 && a.vignette == 0.0 {
        return;
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    if a.sharpness > 0.0 && w > 2 && h > 2 {
        let copy = img.as_raw().clone();
        let k = (a.sharpness / 100.0) * 1.6;
        img.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
            if y == 0 || y == h - 1 {
                return;
            }
            for x in 1..w - 1 {
                let i = (y * w + x) * 4;
                for ch in 0..3 {
                    let blur = (copy[i + ch - 4] as f32 + copy[i + ch + 4] as f32 + copy[i + ch - w * 4] as f32 + copy[i + ch + w * 4] as f32 + copy[i + ch] as f32 * 4.0) / 8.0;
                    let v = copy[i + ch] as f32 + k * (copy[i + ch] as f32 - blur);
                    row[x * 4 + ch] = v.clamp(0.0, 255.0) as u8;
                }
            }
        });
    }
    let (wm, tn, sh, hl, vg) = (a.warmth * 0.45, a.tint * 0.4, a.shadows / 100.0, a.highlights / 100.0, a.vignette / 100.0);
    if wm == 0.0 && tn == 0.0 && sh == 0.0 && hl == 0.0 && vg == 0.0 {
        return;
    }
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let max_d = cx.hypot(cy);
    img.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let p = &mut row[x * 4..x * 4 + 4];
            let (mut r, mut g, mut b) = (p[0] as f32, p[1] as f32, p[2] as f32);
            if wm != 0.0 { r += wm; b -= wm; }
            if tn != 0.0 { g -= tn; r += tn * 0.35; b += tn * 0.35; }
            if sh != 0.0 || hl != 0.0 {
                let l = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
                let add = sh * 70.0 * (1.0 - l) * (1.0 - l) + hl * 70.0 * l * l;
                r += add; g += add; b += add;
            }
            if vg != 0.0 {
                let dn = (x as f32 - cx).hypot(y as f32 - cy) / max_d;
                let s = ((dn - 0.35) / 0.75).clamp(0.0, 1.0);
                let e = s * s;
                if vg > 0.0 {
                    let m = 1.0 - vg * e * 0.95;
                    r *= m; g *= m; b *= m;
                } else {
                    let m = -vg * e * 0.9;
                    r += (255.0 - r) * m; g += (255.0 - g) * m; b += (255.0 - b) * m;
                }
            }
            p[0] = r.clamp(0.0, 255.0) as u8;
            p[1] = g.clamp(0.0, 255.0) as u8;
            p[2] = b.clamp(0.0, 255.0) as u8;
        }
    });
}

fn boxes_for_gauss(sigma: f32, n: usize) -> Vec<usize> {
    let w_ideal = ((12.0 * sigma * sigma / n as f32) + 1.0).sqrt();
    let mut wl = w_ideal.floor() as usize;
    if wl % 2 == 0 { wl -= 1; }
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - (n * wl * wl) as f32 - (4 * n * wl) as f32 - (3 * n) as f32) / (-4.0 * wl as f32 - 4.0);
    let m = m_ideal.round() as usize;
    (0..n).map(|i| if i < m { wl } else { wu }).collect()
}

fn box_row(src: &[u8], dst: &mut [u8], w: usize, r: usize) {
    let scale = 1.0 / (2 * r + 1) as f32;
    for ch in 0..4 {
        let first = src[ch] as f32;
        let last = src[(w - 1) * 4 + ch] as f32;
        let mut acc = first * r as f32;
        for j in 0..r.min(w) { acc += src[j * 4 + ch] as f32; }
        if r > w { acc += last * (r - w) as f32; }
        for x in 0..w {
            let add = if x + r < w { src[(x + r) * 4 + ch] as f32 } else { last };
            acc += add;
            dst[x * 4 + ch] = (acc * scale + 0.5) as u8;
            let sub = if x >= r { src[(x - r) * 4 + ch] as f32 } else { first };
            acc -= sub;
        }
    }
}

fn transpose(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; src.len()];
    out.par_chunks_mut(h * 4).enumerate().for_each(|(x, col)| {
        for y in 0..h {
            let s = (y * w + x) * 4;
            col[y * 4..y * 4 + 4].copy_from_slice(&src[s..s + 4]);
        }
    });
    out
}

/// Gaussian blur (three box passes, edge pixels repeat). `sigma` is in pixels.
pub fn blur(img: &mut RgbaImage, sigma: f32) {
    if sigma < 0.3 {
        return;
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    let boxes = boxes_for_gauss(sigma.max(0.5), 3);
    let mut data = std::mem::take(img).into_raw();
    for pass in 0..2 {
        let (pw, _ph) = if pass == 0 { (w, h) } else { (h, w) };
        for b in &boxes {
            let r = (b - 1) / 2;
            if r == 0 { continue; }
            let src = data.clone();
            data.par_chunks_mut(pw * 4).zip(src.par_chunks(pw * 4)).for_each(|(d, s)| box_row(s, d, pw, r));
        }
        data = if pass == 0 { transpose(&data, w, h) } else { transpose(&data, h, w) };
    }
    *img = RgbaImage::from_raw(w as u32, h as u32, data).expect("size");
}
