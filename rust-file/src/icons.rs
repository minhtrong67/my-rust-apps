//! Icons without any image assets: Fluent glyphs from the Windows icon font when it exists,
//! plus vector-drawn folder / file / drive icons painted straight with egui's painter.

use crate::model::Kind;
use eframe::egui::{self, epaint::PathShape, Color32, CornerRadius, FontFamily, Painter, Pos2, Rect, RichText, Shape, Stroke, StrokeKind, Vec2};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

static ICON_FONT: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub enum Ic {
    Back,
    Forward,
    Up,
    Refresh,
    Search,
    Settings,
    Copy,
    Cut,
    Paste,
    Delete,
    Rename,
    NewFolder,
    NewFile,
    View,
    Sort,
    Info,
    Tools,
    Home,
    Add,
    Close,
    Chevron,
    Undo,
    More,
    Accept,
    Pin,
    Dual,
}

impl Ic {
    fn code(self) -> char {
        match self {
            Ic::Back => '\u{E72B}',
            Ic::Forward => '\u{E72A}',
            Ic::Up => '\u{E74A}',
            Ic::Refresh => '\u{E72C}',
            Ic::Search => '\u{E721}',
            Ic::Settings => '\u{E713}',
            Ic::Copy => '\u{E8C8}',
            Ic::Cut => '\u{E8C6}',
            Ic::Paste => '\u{E77F}',
            Ic::Delete => '\u{E74D}',
            Ic::Rename => '\u{E8AC}',
            Ic::NewFolder => '\u{E8F4}',
            Ic::NewFile => '\u{E8A5}',
            Ic::View => '\u{E8A9}',
            Ic::Sort => '\u{E8CB}',
            Ic::Info => '\u{E946}',
            Ic::Tools => '\u{E90F}',
            Ic::Home => '\u{E80F}',
            Ic::Add => '\u{E710}',
            Ic::Close => '\u{E711}',
            Ic::Chevron => '\u{E76C}',
            Ic::Undo => '\u{E7A7}',
            Ic::More => '\u{E712}',
            Ic::Accept => '\u{E73E}',
            Ic::Pin => '\u{E718}',
            Ic::Dual => '\u{E7C4}',
        }
    }
    fn fallback(self) -> &'static str {
        match self {
            Ic::Back => "⬅",
            Ic::Forward => "➡",
            Ic::Up => "⬆",
            Ic::Refresh => "⟳",
            Ic::Search => "🔍",
            Ic::Settings => "⚙",
            Ic::Copy => "📋",
            Ic::Cut => "✂",
            Ic::Paste => "📥",
            Ic::Delete => "🗑",
            Ic::Rename => "✏",
            Ic::NewFolder => "📁",
            Ic::NewFile => "+",
            Ic::View => "👁",
            Ic::Sort => "⬍",
            Ic::Info => "i",
            Ic::Tools => "⚒",
            Ic::Home => "🏠",
            Ic::Add => "+",
            Ic::Close => "×",
            Ic::Chevron => ">",
            Ic::Undo => "↩",
            Ic::More => "…",
            Ic::Accept => "✓",
            Ic::Pin => "•",
            Ic::Dual => "||",
        }
    }
}

/// An icon as rich text (Fluent glyph when the Windows icon font is present).
pub fn ic(i: Ic) -> RichText {
    if ICON_FONT.load(Ordering::Relaxed) {
        RichText::new(i.code().to_string()).family(FontFamily::Name("icons".into()))
    } else {
        RichText::new(i.fallback())
    }
}

pub fn has_icon_font() -> bool {
    ICON_FONT.load(Ordering::Relaxed)
}

/// Uses the system UI font (Segoe UI on Windows) and the Fluent icon font when available.
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let win = std::env::var("windir").unwrap_or_else(|_| "C:\\Windows".into());
    let dir = std::path::Path::new(&win).join("Fonts");
    let load = |fonts: &mut egui::FontDefinitions, key: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(b) => {
                fonts.font_data.insert(key.to_owned(), Arc::new(egui::FontData::from_owned(b)));
                true
            }
            Err(_) => false,
        }
    };
    if cfg!(windows) {
        if load(&mut fonts, "ui", "segoeui.ttf") {
            fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "ui".into());
        }
        if load(&mut fonts, "sym", "seguisym.ttf") {
            fonts.families.get_mut(&FontFamily::Proportional).unwrap().push("sym".into());
        }
        let got = load(&mut fonts, "icons", "segoeicons.ttf") || load(&mut fonts, "icons", "segmdl2.ttf");
        if got {
            fonts.families.insert(FontFamily::Name("icons".into()), vec!["icons".into()]);
            ICON_FONT.store(true, Ordering::Relaxed);
        }
    }
    ctx.set_fonts(fonts);
}

pub fn kind_color(k: Kind) -> Color32 {
    match k {
        Kind::Folder => Color32::from_rgb(255, 193, 7),
        Kind::Image => Color32::from_rgb(16, 163, 127),
        Kind::Video => Color32::from_rgb(136, 74, 214),
        Kind::Audio => Color32::from_rgb(227, 68, 140),
        Kind::Document => Color32::from_rgb(0, 120, 212),
        Kind::Archive => Color32::from_rgb(214, 140, 0),
        Kind::Code => Color32::from_rgb(0, 150, 160),
        Kind::Text => Color32::from_rgb(110, 120, 130),
        Kind::Other => Color32::from_rgb(120, 130, 150),
    }
}

fn shade(c: Color32, f: f32) -> Color32 {
    let m = |v: u8| ((v as f32 * f).clamp(0.0, 255.0)) as u8;
    Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}

pub fn paint_folder(p: &Painter, r: Rect, color: Color32) {
    let (w, h) = (r.width(), r.height());
    let at = |fx: f32, fy: f32| Pos2::new(r.min.x + w * fx, r.min.y + h * fy);
    let rad = CornerRadius::same(((w * 0.07) as u8).max(1));
    // back plate with the tab
    p.rect_filled(Rect::from_min_max(at(0.04, 0.16), at(0.42, 0.40)), rad, shade(color, 0.82));
    p.rect_filled(Rect::from_min_max(at(0.04, 0.24), at(0.96, 0.84)), rad, shade(color, 0.82));
    // front plate
    p.rect_filled(Rect::from_min_max(at(0.04, 0.36), at(0.96, 0.86)), rad, color);
    p.rect_filled(Rect::from_min_max(at(0.04, 0.36), at(0.96, 0.43)), CornerRadius::ZERO, shade(color, 1.12));
}

pub fn paint_file(p: &Painter, r: Rect, ext: &str, kind: Kind, dark: bool) {
    let (w, h) = (r.width(), r.height());
    let at = |fx: f32, fy: f32| Pos2::new(r.min.x + w * fx, r.min.y + h * fy);
    let body = Rect::from_min_max(at(0.18, 0.04), at(0.82, 0.96));
    let paper = if dark { Color32::from_rgb(70, 74, 82) } else { Color32::from_rgb(250, 251, 252) };
    let edge = if dark { Color32::from_rgb(120, 126, 136) } else { Color32::from_rgb(170, 176, 186) };
    let rad = CornerRadius::same(((w * 0.07) as u8).max(1));
    p.rect_filled(body, rad, paper);
    p.rect_stroke(body, rad, Stroke::new(1.0, edge), StrokeKind::Inside);
    let c = kind_color(kind);
    // folded corner
    let fold = [at(0.6, 0.04), at(0.82, 0.26), at(0.6, 0.26)];
    p.add(Shape::convex_polygon(fold.to_vec(), shade(c, 1.0).gamma_multiply(0.35), Stroke::NONE));
    // colour band + extension
    let band = Rect::from_min_max(at(0.18, 0.66), at(0.82, 0.96));
    p.rect_filled(band, CornerRadius { nw: 0, ne: 0, sw: rad.sw, se: rad.se }, c);
    if w >= 30.0 && !ext.is_empty() {
        let txt: String = ext.chars().take(4).collect::<String>().to_uppercase();
        let fs = (w * 0.16).clamp(7.0, 16.0);
        p.text(band.center(), egui::Align2::CENTER_CENTER, txt, egui::FontId::proportional(fs), Color32::WHITE);
    } else {
        // small icons: three text lines
        let col = c.gamma_multiply(0.6);
        for i in 0..3 {
            let y = 0.24 + i as f32 * 0.12;
            p.line_segment([at(0.28, y), at(0.7 - (i as f32) * 0.08, y)], Stroke::new((h * 0.04).max(1.0), col));
        }
    }
}

pub fn paint_drive(p: &Painter, r: Rect, color: Color32) {
    let (w, h) = (r.width(), r.height());
    let at = |fx: f32, fy: f32| Pos2::new(r.min.x + w * fx, r.min.y + h * fy);
    let body = Rect::from_min_max(at(0.06, 0.3), at(0.94, 0.72));
    p.rect_filled(body, CornerRadius::same(((w * 0.08) as u8).max(2)), color);
    p.rect_filled(Rect::from_min_max(at(0.06, 0.3), at(0.94, 0.4)), CornerRadius::ZERO, shade(color, 1.15));
    p.circle_filled(at(0.8, 0.58), w * 0.04, Color32::from_rgb(120, 230, 140));
    p.line_segment([at(0.14, 0.58), at(0.5, 0.58)], Stroke::new((h * 0.05).max(1.0), shade(color, 0.6)));
}

pub fn paint_star(p: &Painter, c: Pos2, rad: f32, color: Color32) {
    let pt = |i: usize, rr: f32| {
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        Pos2::new(c.x + rr * a.cos(), c.y + rr * a.sin())
    };
    let inner = rad * 0.42;
    for k in 0..5 {
        let o = pt(k * 2, rad);
        let a = pt(k * 2 + 9, inner);
        let b = pt(k * 2 + 1, inner);
        p.add(Shape::convex_polygon(vec![c, a, o, b], color, Stroke::NONE));
    }
}

pub fn paint_chevron(p: &Painter, c: Pos2, size: f32, color: Color32, down: bool) {
    let s = size * 0.5;
    let pts = if down {
        vec![Pos2::new(c.x - s, c.y - s * 0.5), Pos2::new(c.x, c.y + s * 0.5), Pos2::new(c.x + s, c.y - s * 0.5)]
    } else {
        vec![Pos2::new(c.x - s * 0.5, c.y - s), Pos2::new(c.x + s * 0.5, c.y), Pos2::new(c.x - s * 0.5, c.y + s)]
    };
    p.add(PathShape::line(pts, Stroke::new(1.4, color)));
}

pub fn square(r: Rect, side: f32) -> Rect {
    Rect::from_center_size(r.center(), Vec2::splat(side))
}
