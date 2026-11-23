//! Edit state. All geometry is normalised (0..1) so the live preview and the full resolution export render identically.
use serde::{Deserialize, Serialize};

pub const ADJ_KEYS: [&str; 11] =
    ["brightness", "contrast", "saturation", "warmth", "tint", "highlights", "shadows", "sharpness", "vignette", "hue", "blur"];

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Adj {
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub warmth: f32,
    pub tint: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub sharpness: f32,
    pub vignette: f32,
    pub hue: f32,
    pub blur: f32,
}

impl Adj {
    pub fn is_zero(&self) -> bool {
        *self == Adj::default()
    }
    pub fn get(&self, key: &str) -> f32 {
        match key {
            "brightness" => self.brightness,
            "contrast" => self.contrast,
            "saturation" => self.saturation,
            "warmth" => self.warmth,
            "tint" => self.tint,
            "highlights" => self.highlights,
            "shadows" => self.shadows,
            "sharpness" => self.sharpness,
            "vignette" => self.vignette,
            "hue" => self.hue,
            "blur" => self.blur,
            _ => 0.0,
        }
    }
    pub fn set(&mut self, key: &str, v: f32) {
        match key {
            "brightness" => self.brightness = v,
            "contrast" => self.contrast = v,
            "saturation" => self.saturation = v,
            "warmth" => self.warmth = v,
            "tint" => self.tint = v,
            "highlights" => self.highlights = v,
            "shadows" => self.shadows = v,
            "sharpness" => self.sharpness = v,
            "vignette" => self.vignette = v,
            "hue" => self.hue = v,
            "blur" => self.blur = v,
            _ => {}
        }
    }
}

pub type Pt = [f32; 2];
pub type Rgba = [u8; 4];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ShapeKind {
    Line,
    Arrow,
    Rect,
    Oval,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum BlurMode {
    Pixelate,
    Blur,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Item {
    Stroke { id: u64, pts: Vec<Pt>, color: Rgba, width: f32, highlighter: bool },
    Shape { id: u64, shape: ShapeKind, a: Pt, b: Pt, color: Rgba, width: f32, fill: bool },
    Text { id: u64, text: String, x: f32, y: f32, size: f32, rot: f32, color: Rgba, bold: bool, italic: bool, bg: bool },
    Emoji { id: u64, ch: String, x: f32, y: f32, size: f32, rot: f32 },
    Blur { id: u64, x: f32, y: f32, w: f32, h: f32, mode: BlurMode },
}

impl Item {
    pub fn id(&self) -> u64 {
        match self {
            Item::Stroke { id, .. } | Item::Shape { id, .. } | Item::Text { id, .. } | Item::Emoji { id, .. } | Item::Blur { id, .. } => *id,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Default for Crop {
    fn default() -> Self {
        Crop { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EditState {
    /// Quarter turns clockwise (0..3).
    pub rot: u8,
    pub flip_h: bool,
    pub flip_v: bool,
    /// Straighten angle in degrees.
    pub angle: f32,
    pub crop: Crop,
    pub adj: Adj,
    pub filter: String,
    /// 0..100
    pub filter_amt: f32,
    pub items: Vec<Item>,
}

impl Default for EditState {
    fn default() -> Self {
        EditState { rot: 0, flip_h: false, flip_v: false, angle: 0.0, crop: Crop::default(), adj: Adj::default(), filter: "none".into(), filter_amt: 100.0, items: vec![] }
    }
}

impl EditState {
    /// True when saving would change nothing.
    pub fn is_identity(&self) -> bool {
        *self == EditState::default() || (self.rot % 4 == 0 && !self.flip_h && !self.flip_v && self.angle == 0.0 && self.crop == Crop::default() && self.adj.is_zero() && self.filter == "none" && self.items.is_empty())
    }
}

pub const FILTER_IDS: [&str; 12] = ["none", "vivid", "warm", "cool", "mono", "noir", "sepia", "fade", "dramatic", "sunset", "dream", "film"];
