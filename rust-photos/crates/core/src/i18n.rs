//! English and Vietnamese strings. The JSON files were generated from the original TypeScript dictionaries.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum Lang {
    #[serde(rename = "vi")]
    Vi,
    #[default]
    #[serde(rename = "en")]
    En,
}

impl Lang {
    pub fn id(self) -> &'static str {
        match self { Lang::Vi => "vi", Lang::En => "en" }
    }
    pub fn from_id(s: &str) -> Lang {
        if s.eq_ignore_ascii_case("vi") { Lang::Vi } else { Lang::En }
    }
    /// Vietnamese when the operating system language is Vietnamese, English otherwise.
    pub fn system() -> Lang {
        match sys_locale::get_locale() {
            Some(l) if l.to_lowercase().starts_with("vi") => Lang::Vi,
            _ => Lang::En,
        }
    }
}

type Dict = HashMap<String, String>;
static EN: OnceLock<Dict> = OnceLock::new();
static VI: OnceLock<Dict> = OnceLock::new();

fn dict(lang: Lang) -> &'static Dict {
    match lang {
        Lang::En => EN.get_or_init(|| serde_json::from_str(include_str!("../i18n/en.json")).expect("en.json")),
        Lang::Vi => VI.get_or_init(|| serde_json::from_str(include_str!("../i18n/vi.json")).expect("vi.json")),
    }
}

/// Translation of a key, falling back to English and then to the key itself.
pub fn tr(lang: Lang, key: &str) -> String {
    dict(lang).get(key).or_else(|| dict(Lang::En).get(key)).cloned().unwrap_or_else(|| key.to_string())
}

/// Translation with `{name}` placeholders replaced.
pub fn trf(lang: Lang, key: &str, vars: &[(&str, &str)]) -> String {
    let mut s = tr(lang, key);
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}
