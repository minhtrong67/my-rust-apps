//! Windows accent palette and the accent shades Fluent derives from a base colour.
pub struct Accent {
    pub id: &'static str,
    pub hex: &'static str,
    pub vi: &'static str,
    pub en: &'static str,
}

pub const DEFAULT_ACCENT: &str = "#CE422B";

pub const ACCENTS: [Accent; 19] = [
    Accent { id: "rust-red", hex: "#CE422B", vi: "Rỉ sắt", en: "Rust" },
    Accent { id: "ferris", hex: "#F74C00", vi: "Cam Ferris", en: "Ferris" },
    Accent { id: "copper", hex: "#B7410E", vi: "Đồng", en: "Copper" },
    Accent { id: "sand", hex: "#DEA584", vi: "Cát", en: "Sand" },
    Accent { id: "blue", hex: "#0078D4", vi: "Xanh dương", en: "Blue" },
    Accent { id: "cobalt", hex: "#0063B1", vi: "Xanh cô-ban", en: "Cobalt" },
    Accent { id: "sky", hex: "#0099BC", vi: "Xanh da trời", en: "Sky" },
    Accent { id: "cyan", hex: "#00B7C3", vi: "Lục lam", en: "Cyan" },
    Accent { id: "teal", hex: "#038387", vi: "Xanh mòng két", en: "Teal" },
    Accent { id: "green", hex: "#10893E", vi: "Xanh lá", en: "Green" },
    Accent { id: "iris", hex: "#6B69D6", vi: "Tím xanh", en: "Iris" },
    Accent { id: "violet", hex: "#8764B8", vi: "Tím", en: "Violet" },
    Accent { id: "orchid", hex: "#B146C2", vi: "Phong lan", en: "Orchid" },
    Accent { id: "rose", hex: "#E3008C", vi: "Hồng", en: "Rose" },
    Accent { id: "red", hex: "#D13438", vi: "Đỏ", en: "Red" },
    Accent { id: "rust", hex: "#DA3B01", vi: "Đỏ gạch", en: "Rust" },
    Accent { id: "orange", hex: "#F7630C", vi: "Cam", en: "Orange" },
    Accent { id: "gold", hex: "#FF8C00", vi: "Vàng đậm", en: "Gold" },
    Accent { id: "steel", hex: "#68768A", vi: "Xám thép", en: "Steel" },
];

pub fn parse_hex(hex: &str) -> [u8; 3] {
    let h = hex.trim().trim_start_matches('#');
    let n = u32::from_str_radix(&format!("{:0<6}", &h[..h.len().min(6)]), 16).unwrap_or(0x0078D4);
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}

pub fn to_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn to_hsl(c: [u8; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let (l, d) = ((max + min) / 2.0, max - min);
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r { ((g - b) / d).rem_euclid(6.0) } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    ((h * 60.0).rem_euclid(360.0), s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> [u8; 3] {
    let a = s * l.min(1.0 - l);
    let f = |n: f32| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        l - a * (-1.0f32).max((k - 3.0).min((9.0 - k).min(1.0)))
    };
    [(f(0.0) * 255.0).round() as u8, (f(8.0) * 255.0).round() as u8, (f(4.0) * 255.0).round() as u8]
}

fn lum(c: [u8; 3]) -> f32 {
    let f = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
}

/// Text colour that stays readable on top of an accent fill.
fn on_accent(fill: [u8; 3]) -> [u8; 3] {
    if 1.05 / (lum(fill) + 0.05) >= 4.4 { [255, 255, 255] } else { [0, 0, 0] }
}

pub struct Shades {
    pub fill: [u8; 3],
    pub hover: [u8; 3],
    pub press: [u8; 3],
    pub text: [u8; 3],
    pub on: [u8; 3],
}

/// Fluent derives accent shades from the base colour: darker fills in light mode, lighter fills in dark mode.
pub fn accent_shades(hex: &str, dark: bool) -> Shades {
    let (h, s, l) = to_hsl(parse_hex(hex));
    let base = if dark { l.clamp(0.38, 0.44) } else { l.min(0.4) };
    let step = if dark { [0.045, -0.04] } else { [-0.05, 0.06] };
    let fill = from_hsl(h, s, base);
    let hover = from_hsl(h, s, (base + step[0]).clamp(0.1, 0.92));
    let press = from_hsl(h, s, (base + step[0] * 2.0 + if dark { -0.02 } else { 0.0 }).clamp(0.1, 0.92));
    let text = from_hsl(h, s, if dark { l.max(0.72) } else { l.min(0.3) });
    Shades { fill, hover, press, text, on: on_accent(fill) }
}
