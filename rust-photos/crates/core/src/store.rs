//! Settings and user data (favourites, albums, adjustment presets), stored as JSON in the app config folder.
use crate::edit::Adj;
use crate::i18n::Lang;
use crate::library::SortKey;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Material {
    None,
    #[default]
    Mica,
    MicaAlt,
    Acrylic,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum FontSize {
    Sm,
    #[default]
    Md,
    Lg,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum WindowMode {
    #[default]
    Remember,
    Maximized,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transition {
    #[default]
    Fade,
    Slide,
    Zoom,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Slideshow {
    pub interval: u32,
    pub transition: Transition,
    pub shuffle: bool,
    #[serde(rename = "loop")]
    pub repeat: bool,
}

impl Default for Slideshow {
    fn default() -> Self {
        Slideshow { interval: 4, transition: Transition::Fade, shuffle: false, repeat: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeMode,
    pub accent: String,
    pub material: Material,
    pub nav_compact: bool,
    pub font_size: FontSize,
    pub lang: Lang,
    pub auto_load: bool,
    pub library_path: String,
    pub tile: u32,
    pub sort: SortKey,
    pub group: bool,
    pub confirm_delete: bool,
    pub keep_exif: bool,
    pub quality: u8,
    pub reduce_motion: bool,
    pub slide: Slideshow,
    pub window_mode: WindowMode,
    /// Last window size, used by "remember window size".
    pub window_w: u32,
    pub window_h: u32,
    pub window_max: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeMode::System,
            accent: "#0078D4".into(),
            material: Material::Mica,
            nav_compact: false,
            font_size: FontSize::Md,
            lang: Lang::system(),
            auto_load: true,
            library_path: String::new(),
            tile: 190,
            sort: SortKey::DateDesc,
            group: true,
            confirm_delete: true,
            keep_exif: true,
            quality: 95,
            reduce_motion: false,
            slide: Slideshow::default(),
            window_mode: WindowMode::Remember,
            window_w: 1280,
            window_h: 820,
            window_max: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub paths: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AdjPreset {
    pub id: String,
    pub name: String,
    pub adj: Adj,
    pub filter: String,
    pub amount: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct UserData {
    pub favs: Vec<String>,
    pub albums: Vec<Album>,
    pub presets: Vec<AdjPreset>,
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("RustPhotos")
}

fn load<T: for<'a> Deserialize<'a> + Default>(name: &str) -> T {
    std::fs::read_to_string(config_dir().join(name)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save<T: Serialize>(name: &str, v: &T) -> Result<(), String> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    crate::files::write_atomic(&dir.join(name), json.as_bytes())
}

impl Settings {
    pub fn load() -> Settings {
        let first_run = !config_dir().join("settings.json").exists();
        let mut s: Settings = load("settings.json");
        if first_run {
            // The Tauri version stored the window mode in its own folder; keep that choice.
            if let Some(old) = dirs::config_dir().map(|d| d.join("com.minhtrong67.materialphoto").join("settings.json")) {
                if let Some(v) = std::fs::read_to_string(old).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
                    if v.get("window_mode").and_then(|m| m.as_str()) == Some("maximized") {
                        s.window_mode = WindowMode::Maximized;
                    }
                }
            }
        }
        s
    }
    pub fn save(&self) -> Result<(), String> { save("settings.json", self) }
}

impl UserData {
    pub fn load() -> UserData { load("data.json") }
    pub fn save(&self) -> Result<(), String> { save("data.json", self) }

    pub fn is_fav(&self, path: &str) -> bool { self.favs.iter().any(|p| p == path) }

    pub fn toggle_fav(&mut self, path: &str) {
        if self.is_fav(path) { self.favs.retain(|p| p != path); } else { self.favs.insert(0, path.to_string()); }
    }

    /// Keeps favourites and albums pointing at the right file after a rename or a move.
    pub fn repath(&mut self, from: &str, to: &str) {
        for p in self.favs.iter_mut().filter(|p| *p == from) { *p = to.to_string(); }
        for a in self.albums.iter_mut() { for p in a.paths.iter_mut().filter(|p| *p == from) { *p = to.to_string(); } }
    }

    pub fn forget(&mut self, paths: &[String]) {
        self.favs.retain(|p| !paths.contains(p));
    }
}
