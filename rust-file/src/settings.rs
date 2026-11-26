//! Persistent settings (one small JSON file in the user's config folder).

use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    Details,
    Small,
    Medium,
    Large,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Modified,
    Kind,
    Size,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct FolderView {
    pub mode: ViewMode,
    pub key: SortKey,
    pub asc: bool,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct CustomCmd {
    pub name: String,
    pub program: String,
    /// space separated; `{path}` and `{dir}` are replaced
    pub args: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub lang: String,
    pub theme: String,
    pub accent: [u8; 3],
    pub accent_auto: bool,
    pub text_scale: f32,
    pub show_hidden: bool,
    pub show_ext: bool,
    pub compact: bool,
    pub details_pane: bool,
    pub dual: bool,
    pub mode: ViewMode,
    pub sort_key: SortKey,
    pub sort_asc: bool,
    pub confirm_delete: bool,
    pub folder_sizes: bool,
    pub restore_tabs: bool,
    pub start_page: bool,
    pub pinned: Vec<String>,
    pub recent: Vec<String>,
    pub tags: HashMap<String, u8>,
    pub stars: HashSet<String>,
    pub tag_names: Vec<String>,
    pub folder_colors: HashMap<String, [u8; 3]>,
    pub per_folder: HashMap<String, FolderView>,
    pub search_history: Vec<String>,
    pub saved_searches: Vec<String>,
    pub custom_cmds: Vec<CustomCmd>,
    pub tabs: Vec<String>,
    pub win_size: [f32; 2],
    pub maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            lang: "en".into(),
            theme: "system".into(),
            accent: [0, 103, 192],
            accent_auto: true,
            text_scale: 1.0,
            show_hidden: false,
            show_ext: true,
            compact: false,
            details_pane: false,
            dual: false,
            mode: ViewMode::Details,
            sort_key: SortKey::Name,
            sort_asc: true,
            confirm_delete: false,
            folder_sizes: false,
            restore_tabs: true,
            start_page: false,
            pinned: Vec::new(),
            recent: Vec::new(),
            tags: HashMap::new(),
            stars: HashSet::new(),
            tag_names: ["Red", "Orange", "Yellow", "Green", "Blue", "Purple", "Gray"].iter().map(|s| s.to_string()).collect(),
            folder_colors: HashMap::new(),
            per_folder: HashMap::new(),
            search_history: Vec::new(),
            saved_searches: Vec::new(),
            custom_cmds: Vec::new(),
            tabs: Vec::new(),
            win_size: [1120.0, 720.0],
            maximized: false,
        }
    }
}

pub const TAG_COLORS: [[u8; 3]; 7] = [
    [232, 17, 35],
    [247, 99, 12],
    [255, 185, 0],
    [16, 124, 16],
    [0, 120, 212],
    [136, 23, 152],
    [118, 118, 118],
];

pub const ACCENTS: [(&str, [u8; 3]); 12] = [
    ("Blue", [0, 103, 192]),
    ("Sky", [0, 153, 188]),
    ("Teal", [0, 133, 119]),
    ("Green", [16, 137, 62]),
    ("Lime", [118, 163, 0]),
    ("Amber", [255, 140, 0]),
    ("Orange", [247, 99, 12]),
    ("Red", [232, 17, 35]),
    ("Pink", [227, 0, 140]),
    ("Purple", [136, 23, 152]),
    ("Indigo", [88, 80, 224]),
    ("Slate", [105, 121, 126]),
];

fn file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("rust-file").join("settings.json"))
}

impl Settings {
    pub fn load() -> Settings {
        file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(f) = file() {
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Ok(t) = serde_json::to_string_pretty(self) {
                let tmp = f.with_extension("tmp");
                if std::fs::write(&tmp, t).is_ok() {
                    let _ = std::fs::rename(&tmp, &f);
                }
            }
        }
    }

    pub fn tag_of(&self, p: &std::path::Path) -> Option<u8> {
        self.tags.get(p.to_string_lossy().as_ref()).copied()
    }
    pub fn starred(&self, p: &std::path::Path) -> bool {
        self.stars.contains(p.to_string_lossy().as_ref())
    }
    pub fn push_recent(&mut self, p: &str) {
        self.recent.retain(|x| x != p);
        self.recent.insert(0, p.to_string());
        self.recent.truncate(24);
    }
}
