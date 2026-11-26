//! Dialogs, command palette and the disk-usage treemap.

use crate::{
    app::*,
    fsops::*,
    i18n::{self, t},
    icons::*,
    model::*,
    settings::*,
};
use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
};

/* ------------------------------------------------------------------- state */

pub enum Dialog {
    None,
    Conflict { sources: Vec<PathBuf>, dest: PathBuf, mv: bool, names: Vec<String> },
    Delete { paths: Vec<PathBuf>, perm: bool },
    Props { path: PathBuf, props: Option<Props> },
    Batch(BatchState),
    AdvSearch(AdvState),
    Dupes(DupState),
    Usage(UsageState),
    Settings(u8),
    Compress { paths: Vec<PathBuf>, dest: PathBuf, name: String, kind: usize },
    About,
}

pub struct BatchState {
    pub paths: Vec<PathBuf>,
    pub mode: usize,
    pub pattern: String,
    pub find: String,
    pub repl: String,
    pub regex: bool,
    pub case: usize,
    pub rand_kind: usize,
    pub rand_len: usize,
    pub seed: u64,
}

impl BatchState {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        BatchState {
            paths,
            mode: 0,
            pattern: "{name} {n}".into(),
            find: String::new(),
            repl: String::new(),
            regex: false,
            case: 0,
            rand_kind: 2,
            rand_len: 8,
            seed: now_ms() as u64,
        }
    }
}

#[derive(Default)]
pub struct AdvState {
    pub query: String,
    pub regex: bool,
    pub exts: String,
    pub min_mb: String,
    pub max_mb: String,
    pub days: String,
}

pub struct DupState {
    pub root: PathBuf,
    pub min_kb: String,
    pub scanning: bool,
    pub groups: Vec<DupGroup>,
    pub picked: HashSet<PathBuf>,
    pub scanned: bool,
}

pub struct UsageState {
    pub path: PathBuf,
    pub loading: bool,
    pub items: Vec<SizeItem>,
    pub error: Option<String>,
    pub stack: Vec<PathBuf>,
}

pub struct PaletteState {
    pub query: String,
    pub sel: usize,
    pub focus_set: bool,
}

/* ------------------------------------------------------------ batch rename */

fn guid(r: &mut Rng) -> String {
    let h = |r: &mut Rng, n: usize| (0..n).map(|_| r.pick("0123456789abcdef")).collect::<String>();
    format!("{}-{}-4{}-a{}-{}", h(r, 8), h(r, 4), h(r, 3), h(r, 3), h(r, 12))
}

pub fn batch_names(st: &BatchState) -> Vec<String> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut rng = Rng::new();
    for _ in 0..(st.seed % 7) {
        rng.next();
    }
    let re = if st.mode == 1 && st.regex && !st.find.is_empty() { regex::Regex::new(&st.find).ok() } else { None };
    st.paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (stem, ext) = split_name(&name, p.is_dir());
            let mut out = match st.mode {
                0 => {
                    let mut s = st.pattern.replace("{name}", &stem).replace("{date}", &today);
                    // {n} and {n:3}
                    let mut res = String::new();
                    let mut rest = s.as_str();
                    while let Some(pos) = rest.find("{n") {
                        res.push_str(&rest[..pos]);
                        let after = &rest[pos + 2..];
                        if let Some(end) = after.find('}') {
                            let spec = &after[..end];
                            let width = spec.strip_prefix(':').and_then(|w| w.parse::<usize>().ok()).unwrap_or(1);
                            res.push_str(&format!("{:0width$}", i + 1, width = width));
                            rest = &after[end + 1..];
                        } else {
                            res.push_str("{n");
                            rest = after;
                        }
                    }
                    res.push_str(rest);
                    s = res;
                    s
                }
                1 => {
                    if st.find.is_empty() {
                        stem.clone()
                    } else if let Some(r) = &re {
                        r.replace_all(&stem, st.repl.as_str()).into_owned()
                    } else if st.regex {
                        stem.clone()
                    } else {
                        stem.replace(&st.find, &st.repl)
                    }
                }
                _ => {
                    let n = st.rand_len.clamp(1, 64);
                    match st.rand_kind {
                        0 => (0..n).map(|_| rng.pick("abcdefghijklmnopqrstuvwxyz")).collect(),
                        1 => (0..n).map(|_| rng.pick("0123456789")).collect(),
                        2 => (0..n).map(|_| rng.pick("abcdefghijklmnopqrstuvwxyz0123456789")).collect(),
                        3 => guid(&mut rng),
                        _ => format!("{}-{}", today, (0..6).map(|_| rng.pick("abcdefghijklmnopqrstuvwxyz0123456789")).collect::<String>()),
                    }
                }
            };
            out = match st.case {
                1 => out.to_lowercase(),
                2 => out.to_uppercase(),
                3 => out
                    .split(' ')
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                            None => String::new(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => out,
            };
            // the old extension is kept unless the pattern writes its own
            if st.mode == 0 && st.pattern.contains('.') && !p.is_dir() {
                out
            } else {
                format!("{out}{ext}")
            }
        })
        .collect()
}

fn small_label(ui: &mut egui::Ui, s: &str) {
    ui.label(RichText::new(s).weak().small());
}

impl App {
    fn apply_batch(&mut self, st: &BatchState) {
        let names = batch_names(st);
        let mut tmp: Vec<(PathBuf, PathBuf, String)> = Vec::new();
        for (p, n) in st.paths.iter().zip(names.iter()) {
            if !valid_name(n) {
                self.err(App::friendly("invalid-name"));
                return;
            }
            tmp.push((p.clone(), p.parent().unwrap_or(Path::new("")).join(n.trim()), n.clone()));
        }
        // phase 1: move everything to temporary names so swaps cannot collide
        let mut staged: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new();
        for (i, (from, to, _)) in tmp.iter().enumerate() {
            if from == to {
                continue;
            }
            let t1 = from.with_file_name(format!(".rf-tmp-{}-{}", now_ms(), i));
            if std::fs::rename(from, &t1).is_ok() {
                staged.push((from.clone(), t1, to.clone()));
            }
        }
        let mut pairs = Vec::new();
        let mut finals = Vec::new();
        for (from, t1, to) in staged {
            let target = if to.exists() {
                let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                free_name(to.parent().unwrap_or(Path::new("")), &name, to.is_dir(), false)
            } else {
                to
            };
            if std::fs::rename(&t1, &target).is_ok() {
                pairs.push((from, target.clone()));
                finals.push(target);
            } else {
                let _ = std::fs::rename(&t1, &from);
            }
        }
        if !pairs.is_empty() {
            self.undo.push(Undo::Move { pairs: pairs.clone() });
            let p = self.fp();
            self.panes[p].tab_mut().select_after = finals;
            self.ok(&format!("{} - {} {}", t("Renamed"), pairs.len(), t("items")));
        }
        self.reload_all();
    }

    /* ---------------------------------------------------------------- dialogs */

    pub fn show_dialogs(&mut self, ctx: &egui::Context) {
        let mut d = std::mem::replace(&mut self.dialog, Dialog::None);
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !matches!(d, Dialog::None);
        let win = |title: &str| egui::Window::new(RichText::new(title).strong()).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]);
        match &mut d {
            Dialog::None => {}
            Dialog::Conflict { sources, dest, mv, names } => {
                let mut choice: Option<Policy> = None;
                win(t("Name conflict")).show(ctx, |ui| {
                    ui.set_width(380.0);
                    ui.label(format!("{} {}", names.len(), t("already exist in the destination.")));
                    ui.add_space(4.0);
                    for n in names.iter().take(6) {
                        ui.label(RichText::new(n).strong());
                    }
                    if names.len() > 6 {
                        ui.label(format!("+ {}", names.len() - 6));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(t("Replace")).clicked() {
                            choice = Some(Policy::Replace);
                        }
                        if ui.button(t("Keep both")).clicked() {
                            choice = Some(Policy::Rename);
                        }
                        if ui.button(t("Skip")).clicked() {
                            choice = Some(Policy::Skip);
                        }
                        if ui.button(t("Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
                if let Some(pol) = choice {
                    let (s, de, m) = (sources.clone(), dest.clone(), *mv);
                    self.start_paste(s, de, m, pol);
                    if m {
                        self.clip = None;
                    }
                    close = true;
                }
            }
            Dialog::Delete { paths, perm } => {
                let mut yes = false;
                win(t("Delete items?")).show(ctx, |ui| {
                    ui.set_width(340.0);
                    ui.label(format!("{} {}", paths.len(), t("items")));
                    for p in paths.iter().take(4) {
                        ui.label(RichText::new(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).strong());
                    }
                    ui.label(if *perm { t("This cannot be undone.") } else { t("Move to the Recycle Bin?") });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new(t("Delete")).color(Color32::WHITE)).fill(Color32::from_rgb(196, 43, 28))).clicked() {
                            yes = true;
                        }
                        if ui.button(t("Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
                if yes {
                    let (p, pm) = (paths.clone(), *perm);
                    self.start_delete(p, pm);
                    close = true;
                }
            }
            Dialog::Props { path, props } => {
                let mut open = true;
                win(t("Properties")).open(&mut open).show(ctx, |ui| {
                    ui.set_width(380.0);
                    match props {
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(t("Calculating..."));
                            });
                            ui.label(path.display().to_string());
                        }
                        Some(p) => {
                            ui.label(RichText::new(&p.name).heading());
                            ui.add_space(4.0);
                            egui::Grid::new("props").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                                let row = |ui: &mut egui::Ui, k: &str, v: String| {
                                    ui.label(RichText::new(k).weak());
                                    ui.label(v);
                                    ui.end_row();
                                };
                                row(ui, t("Location"), p.path.parent().map(|x| x.display().to_string()).unwrap_or_default());
                                row(ui, t("Size"), format!("{} ({} B)", fmt_size(p.size), p.size));
                                if p.is_dir {
                                    row(ui, t("Contains"), format!("{} {}, {} {}", p.files, t("files"), p.folders, t("folders")));
                                }
                                row(ui, t("Created"), fmt_date(p.created));
                                row(ui, t("Modified"), fmt_date(p.modified));
                                row(ui, t("Accessed"), fmt_date(p.accessed));
                                let mut attrs = Vec::new();
                                if p.readonly {
                                    attrs.push(t("Read-only"));
                                }
                                if p.hidden {
                                    attrs.push(t("Hidden"));
                                }
                                if !attrs.is_empty() {
                                    row(ui, "", attrs.join(", "));
                                }
                            });
                        }
                    }
                });
                if !open {
                    close = true;
                }
            }
            Dialog::Batch(st) => {
                let mut apply = false;
                let mut open = true;
                win(t("Batch rename")).open(&mut open).show(ctx, |ui| {
                    ui.set_width(520.0);
                    ui.horizontal(|ui| {
                        for (i, l) in [t("Pattern"), t("Find & replace"), t("Random")].iter().enumerate() {
                            ui.selectable_value(&mut st.mode, i, *l);
                        }
                    });
                    ui.add_space(6.0);
                    match st.mode {
                        0 => {
                            ui.add(egui::TextEdit::singleline(&mut st.pattern).desired_width(f32::INFINITY));
                            small_label(ui, "{name}  {n}  {n:3}  {date}");
                        }
                        1 => {
                            ui.horizontal(|ui| {
                                ui.label(t("Find"));
                                ui.text_edit_singleline(&mut st.find);
                            });
                            ui.horizontal(|ui| {
                                ui.label(t("Replace with"));
                                ui.text_edit_singleline(&mut st.repl);
                            });
                            ui.checkbox(&mut st.regex, t("Use regex"));
                        }
                        _ => {
                            ui.horizontal(|ui| {
                                for (i, l) in [t("Letters"), t("Digits"), t("Letters + digits"), t("GUID"), t("Date + random")].iter().enumerate() {
                                    ui.selectable_value(&mut st.rand_kind, i, *l);
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label(t("Length"));
                                ui.add(egui::Slider::new(&mut st.rand_len, 3..=32));
                                if ui.button(ic(Ic::Refresh)).clicked() {
                                    st.seed = st.seed.wrapping_add(1);
                                }
                            });
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label(t("Letter case"));
                        for (i, l) in [t("Keep"), t("lowercase"), t("UPPERCASE"), t("Title Case")].iter().enumerate() {
                            ui.selectable_value(&mut st.case, i, *l);
                        }
                    });
                    ui.separator();
                    ui.label(RichText::new(t("Preview")).strong());
                    let names = batch_names(st);
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        for (p, n) in st.paths.iter().zip(names.iter()).take(200) {
                            let old = p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(old).weak());
                                ui.label("->");
                                ui.label(RichText::new(n).strong());
                            });
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new(t("Apply")).color(Color32::WHITE)).fill(ui.visuals().selection.bg_fill)).clicked() {
                            apply = true;
                        }
                        if ui.button(t("Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
                if !open {
                    close = true;
                }
                if apply {
                    let snapshot = BatchState { paths: st.paths.clone(), pattern: st.pattern.clone(), find: st.find.clone(), repl: st.repl.clone(), ..*st };
                    self.apply_batch(&snapshot);
                    close = true;
                }
            }
            Dialog::AdvSearch(st) => {
                let mut go = false;
                let mut open = true;
                let mut pick: Option<String> = None;
                win(t("Advanced search")).open(&mut open).show(ctx, |ui| {
                    ui.set_width(440.0);
                    ui.horizontal(|ui| {
                        ui.label(ic(Ic::Search));
                        let r = ui.add(egui::TextEdit::singleline(&mut st.query).desired_width(f32::INFINITY).hint_text(t("Search")));
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            go = true;
                        }
                    });
                    ui.checkbox(&mut st.regex, t("Use regular expression"));
                    egui::Grid::new("adv").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label(t("Extensions (e.g. png, jpg)"));
                        ui.text_edit_singleline(&mut st.exts);
                        ui.end_row();
                        ui.label(t("Min size (MB)"));
                        ui.text_edit_singleline(&mut st.min_mb);
                        ui.end_row();
                        ui.label(t("Max size (MB)"));
                        ui.text_edit_singleline(&mut st.max_mb);
                        ui.end_row();
                        ui.label(t("Modified in the last (days)"));
                        ui.text_edit_singleline(&mut st.days);
                        ui.end_row();
                    });
                    if !self.s.search_history.is_empty() {
                        ui.separator();
                        ui.label(RichText::new(t("Search history")).strong());
                        ui.horizontal_wrapped(|ui| {
                            for q in self.s.search_history.iter().take(8) {
                                if ui.small_button(q).clicked() {
                                    pick = Some(q.clone());
                                }
                            }
                        });
                    }
                    if !self.s.saved_searches.is_empty() {
                        ui.label(RichText::new(t("Saved searches")).strong());
                        ui.horizontal_wrapped(|ui| {
                            for q in self.s.saved_searches.iter() {
                                if ui.small_button(q).clicked() {
                                    pick = Some(q.clone());
                                }
                            }
                        });
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new(t("Search")).color(Color32::WHITE)).fill(ui.visuals().selection.bg_fill)).clicked() {
                            go = true;
                        }
                        if ui.button(t("Save this search")).clicked() && !st.query.trim().is_empty() {
                            let q = st.query.clone();
                            if !self.s.saved_searches.contains(&q) {
                                self.s.saved_searches.push(q);
                                self.dirty = true;
                            }
                        }
                    });
                });
                if let Some(q) = pick {
                    st.query = q;
                }
                if !open {
                    close = true;
                }
                if go {
                    let exts: Vec<String> = st.exts.split([',', ' ', ';']).map(|e| e.trim().trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty()).collect();
                    let mb = |s: &str| s.trim().replace(',', ".").parse::<f64>().ok().map(|v| (v * 1024.0 * 1024.0) as u64);
                    let after = st.days.trim().parse::<i64>().ok().map(|dd| now_ms() - dd * 86_400_000);
                    let opts = SearchOpts { regex: st.regex, exts, min_size: mb(&st.min_mb), max_size: mb(&st.max_mb), after, before: None };
                    let q = st.query.clone();
                    let p = self.fp();
                    self.start_search(p, q, opts);
                    close = true;
                }
            }
            Dialog::Dupes(st) => {
                let mut open = true;
                let mut scan = false;
                let mut trash_sel: Option<Vec<PathBuf>> = None;
                win(t("Duplicate finder")).open(&mut open).resizable(true).default_size([620.0, 480.0]).show(ctx, |ui| {
                    ui.label(RichText::new(st.root.display().to_string()).weak());
                    ui.horizontal(|ui| {
                        ui.label(t("Minimum size (KB)"));
                        ui.add(egui::TextEdit::singleline(&mut st.min_kb).desired_width(70.0));
                        if st.scanning {
                            ui.spinner();
                            ui.label(t("Scanning..."));
                        } else if ui.button(t("Scan")).clicked() {
                            scan = true;
                        }
                    });
                    if st.scanned && !st.scanning {
                        let wasted: u64 = st.groups.iter().map(|g| g.size * (g.files.len() as u64 - 1)).sum();
                        ui.label(format!("{} {} - {} {}", st.groups.len(), t("duplicate groups"), fmt_size(wasted), t("wasted")));
                        ui.horizontal(|ui| {
                            if ui.button(t("Keep first, select the rest")).clicked() {
                                st.picked = st.groups.iter().flat_map(|g| g.files.iter().skip(1).cloned()).collect();
                            }
                            if !st.picked.is_empty() && ui.button(format!("{} ({})", t("Move selected to Recycle Bin"), st.picked.len())).clicked() {
                                trash_sel = Some(st.picked.iter().cloned().collect());
                            }
                        });
                    }
                    ui.separator();
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        for g in st.groups.iter().take(300) {
                            ui.label(RichText::new(format!("{} × {}", fmt_size(g.size), g.files.len())).strong());
                            for f in &g.files {
                                let mut on = st.picked.contains(f);
                                if ui.checkbox(&mut on, f.display().to_string()).changed() {
                                    if on {
                                        st.picked.insert(f.clone());
                                    } else {
                                        st.picked.remove(f);
                                    }
                                }
                            }
                            ui.add_space(6.0);
                        }
                    });
                });
                if scan {
                    st.scanning = true;
                    st.scanned = true;
                    st.groups.clear();
                    let min = st.min_kb.trim().parse::<u64>().unwrap_or(0) * 1024;
                    let (root, bus) = (st.root.clone(), self.bus());
                    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                    self.dup_cancel = cancel.clone();
                    std::thread::spawn(move || {
                        let g = find_duplicates(&root, min, &cancel);
                        bus.send(Msg::Dupes(g));
                    });
                }
                if let Some(v) = trash_sel {
                    st.picked.clear();
                    let gone: HashSet<PathBuf> = v.iter().cloned().collect();
                    for g in st.groups.iter_mut() {
                        g.files.retain(|f| !gone.contains(f));
                    }
                    st.groups.retain(|g| g.files.len() > 1);
                    self.start_delete(v, false);
                }
                if !open {
                    self.dup_cancel.store(true, Ordering::Relaxed);
                    close = true;
                }
            }
            Dialog::Usage(u) => {
                let mut open = true;
                let mut goto: Option<PathBuf> = None;
                let mut back = false;
                win(t("Disk usage")).open(&mut open).resizable(true).default_size([760.0, 540.0]).show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if !u.stack.is_empty() && ui.button(ic(Ic::Back)).clicked() {
                            back = true;
                        }
                        ui.label(RichText::new(u.path.display().to_string()).strong());
                        if u.loading {
                            ui.spinner();
                            ui.label(t("Analyzing..."));
                        }
                    });
                    if let Some(e) = &u.error {
                        ui.colored_label(Color32::from_rgb(196, 43, 28), e);
                    }
                    let avail = ui.available_size();
                    let (rect, _) = ui.allocate_exact_size(vec2(avail.x, (avail.y - 4.0).max(200.0)), Sense::hover());
                    if !u.items.is_empty() {
                        goto = draw_treemap(ui, rect, &u.items);
                    }
                });
                if !open {
                    close = true;
                }
                let load = |this: &mut App, path: PathBuf, u: &mut UsageState| {
                    u.path = path.clone();
                    u.loading = true;
                    u.items.clear();
                    u.error = None;
                    let bus = this.bus();
                    std::thread::spawn(move || {
                        let never = std::sync::atomic::AtomicBool::new(false);
                        let res = folder_sizes(&path, &never);
                        bus.send(Msg::Sizes { path, res });
                    });
                };
                if let Some(g) = goto {
                    let old = u.path.clone();
                    u.stack.push(old);
                    load(self, g, u);
                } else if back {
                    if let Some(prev) = u.stack.pop() {
                        load(self, prev, u);
                    }
                }
            }
            Dialog::Settings(tab) => {
                let mut open = true;
                let mut changed = false;
                let mut lang_changed = false;
                win(t("Settings")).open(&mut open).resizable(true).default_size([520.0, 520.0]).show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        for (i, l) in [t("General"), t("Appearance"), t("Context menu")].iter().enumerate() {
                            ui.selectable_value(tab, i as u8, *l);
                        }
                    });
                    ui.separator();
                    match *tab {
                        0 => {
                            ui.horizontal(|ui| {
                                ui.label(t("Language"));
                                let cur = if self.s.lang == "vi" { "Tiếng Việt" } else { "English" };
                                egui::ComboBox::from_id_salt("lang").selected_text(cur).show_ui(ui, |ui| {
                                    if ui.selectable_label(self.s.lang == "en", "English").clicked() {
                                        self.s.lang = "en".into();
                                        lang_changed = true;
                                    }
                                    if ui.selectable_label(self.s.lang == "vi", "Tiếng Việt").clicked() {
                                        self.s.lang = "vi".into();
                                        lang_changed = true;
                                    }
                                });
                            });
                            changed |= ui.checkbox(&mut self.s.show_hidden, t("Show hidden files")).changed();
                            changed |= ui.checkbox(&mut self.s.show_ext, t("File extensions")).changed();
                            changed |= ui.checkbox(&mut self.s.compact, t("Compact rows")).changed();
                            changed |= ui.checkbox(&mut self.s.confirm_delete, t("Confirm before deleting")).changed();
                            changed |= ui.checkbox(&mut self.s.folder_sizes, t("Show folder sizes")).changed();
                            changed |= ui.checkbox(&mut self.s.restore_tabs, t("Restore tabs on start")).changed();
                            changed |= ui.checkbox(&mut self.s.start_page, t("Start on This PC page")).changed();
                        }
                        1 => {
                            ui.horizontal(|ui| {
                                ui.label(t("Theme"));
                                for (k, l) in [("system", t("System")), ("light", t("Light")), ("dark", t("Dark"))] {
                                    if ui.selectable_label(self.s.theme == k, l).clicked() {
                                        self.s.theme = k.into();
                                        changed = true;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label(t("Text size"));
                                changed |= ui.add(egui::Slider::new(&mut self.s.text_scale, 0.8..=1.5).fixed_decimals(2)).changed();
                            });
                            ui.add_space(4.0);
                            if cfg!(windows) {
                                changed |= ui.checkbox(&mut self.s.accent_auto, t("Use Windows accent color")).changed();
                            }
                            ui.label(t("Accent color"));
                            ui.horizontal_wrapped(|ui| {
                                for (_, c) in ACCENTS.iter() {
                                    let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                                    ui.painter().rect_filled(r, CornerRadius::same(6), Color32::from_rgb(c[0], c[1], c[2]));
                                    if self.s.accent == *c && !self.s.accent_auto {
                                        ui.painter().rect_stroke(r.expand(2.0), CornerRadius::same(7), Stroke::new(2.0, ui.visuals().text_color()), StrokeKind::Outside);
                                    }
                                    if resp.clicked() {
                                        self.s.accent = *c;
                                        self.s.accent_auto = false;
                                        changed = true;
                                    }
                                }
                                let mut col = Color32::from_rgb(self.s.accent[0], self.s.accent[1], self.s.accent[2]);
                                if ui.color_edit_button_srgba(&mut col).changed() {
                                    self.s.accent = [col.r(), col.g(), col.b()];
                                    self.s.accent_auto = false;
                                    changed = true;
                                }
                            });
                        }
                        _ => {
                            ui.label(RichText::new(t("Custom commands")).strong());
                            let mut remove: Option<usize> = None;
                            for (i, c) in self.s.custom_cmds.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    changed |= ui.add(egui::TextEdit::singleline(&mut c.name).hint_text(t("Name")).desired_width(110.0)).changed();
                                    changed |= ui.add(egui::TextEdit::singleline(&mut c.program).hint_text(t("Program")).desired_width(140.0)).changed();
                                    changed |= ui.add(egui::TextEdit::singleline(&mut c.args).hint_text(t("Arguments ({path} {dir})")).desired_width(160.0)).changed();
                                    if ui.button(ic(Ic::Delete)).clicked() {
                                        remove = Some(i);
                                    }
                                });
                            }
                            if let Some(i) = remove {
                                self.s.custom_cmds.remove(i);
                                changed = true;
                            }
                            if ui.button(format!("+ {}", t("Add command"))).clicked() {
                                self.s.custom_cmds.push(CustomCmd { name: "Notepad++".into(), program: "notepad++".into(), args: "{path}".into() });
                                changed = true;
                            }
                        }
                    }
                });
                if lang_changed {
                    i18n::set_vi(self.s.lang == "vi");
                    changed = true;
                }
                if changed {
                    self.dirty = true;
                    self.style_sig = None;
                    for p in 0..self.panes.len() {
                        for a in 0..self.panes[p].tabs.len() {
                            let s = &self.s;
                            self.panes[p].tabs[a].rebuild(s);
                        }
                    }
                }
                if !open {
                    close = true;
                }
            }
            Dialog::Compress { paths, dest, name, kind } => {
                let mut go = false;
                let mut open = true;
                win(t("Compress")).open(&mut open).show(ctx, |ui| {
                    ui.set_width(340.0);
                    ui.horizontal(|ui| {
                        ui.label(t("Archive name"));
                        ui.text_edit_singleline(name);
                    });
                    ui.horizontal(|ui| {
                        ui.label(t("Format"));
                        ui.selectable_value(kind, 0, "ZIP");
                        ui.selectable_value(kind, 1, "RAR");
                    });
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new(t("Create")).color(Color32::WHITE)).fill(ui.visuals().selection.bg_fill)).clicked() {
                            go = true;
                        }
                        if ui.button(t("Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
                if !open {
                    close = true;
                }
                if go && valid_name(name) {
                    let (ps, de, nm, k) = (paths.clone(), dest.clone(), name.trim().to_string(), if *kind == 0 { "zip" } else { "rar" });
                    let bus = self.bus();
                    self.op = Some(Op { shared: Arc::new(Shared::default()), label: t("Compress").into() });
                    std::thread::spawn(move || {
                        let res = compress_items(&ps, &de, &nm, k);
                        bus.send(Msg::Archive { res, extracted: false });
                    });
                    close = true;
                }
            }
            Dialog::About => {
                let mut open = true;
                win(t("About")).open(&mut open).show(ctx, |ui| {
                    ui.set_width(320.0);
                    ui.vertical_centered(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                        paint_folder(ui.painter(), r, Color32::from_rgb(255, 193, 7));
                        ui.label(RichText::new("Rust File").heading());
                        ui.label(t("Pure Rust native file manager"));
                        ui.label(RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).weak());
                    });
                });
                if !open {
                    close = true;
                }
            }
        }
        if !close && self.dialog_is_none() {
            self.dialog = d;
        }
    }

    fn dialog_is_none(&self) -> bool {
        matches!(self.dialog, Dialog::None)
    }

    /* ---------------------------------------------------------- palette */

    pub fn show_palette(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.palette.take() else { return };
        let p = self.fp();
        // candidate list
        let mut items: Vec<(String, String, Act)> = Vec::new(); // (label, hint, act)
        let cmd = |items: &mut Vec<(String, String, Act)>, l: &str, a: Act| items.push((l.to_string(), t("Toggle").to_string(), a));
        cmd(&mut items, t("New tab"), Act::NewTab(Loc::ThisPc));
        cmd(&mut items, t("Dual pane"), Act::ToggleDual);
        cmd(&mut items, t("Settings"), Act::Dialog(Dialog::Settings(0)));
        cmd(&mut items, t("Advanced search"), Act::Dialog(Dialog::AdvSearch(AdvState::default())));
        if let Some(d) = self.panes[p].tab().cur_dir() {
            cmd(
                &mut items,
                t("Duplicate finder"),
                Act::Dialog(Dialog::Dupes(DupState { root: d.to_path_buf(), min_kb: "1".into(), scanning: false, groups: vec![], picked: HashSet::new(), scanned: false })),
            );
            cmd(&mut items, t("Disk usage"), Act::Usage(d.to_path_buf()));
            cmd(&mut items, t("Terminal here"), Act::OpenWith("terminal", d.to_path_buf()));
        }
        cmd(&mut items, t("Clean temp files"), Act::CleanTemp);
        cmd(&mut items, t("Undo"), Act::Undo);
        cmd(&mut items, t("Show hidden files"), Act::Dialog(Dialog::Settings(0)));
        cmd(&mut items, t("This PC"), Act::Navigate(p, Loc::ThisPc));
        for (n, d) in &self.places {
            items.push((t(n).to_string(), t("Go to").to_string(), Act::Navigate(p, Loc::Dir(d.clone()))));
        }
        for d in &self.drives {
            items.push((d.path.display().to_string(), t("Drives").to_string(), Act::Navigate(p, Loc::Dir(d.path.clone()))));
        }
        for pin in &self.s.pinned {
            items.push((pin.clone(), t("Pin").to_string(), Act::Navigate(p, Loc::Dir(PathBuf::from(pin)))));
        }
        for r in self.s.recent.iter().take(12) {
            items.push((r.clone(), t("Recent").to_string(), Act::Navigate(p, Loc::Dir(PathBuf::from(r)))));
        }
        for e in self.panes[p].tab().entries.iter().take(3000) {
            items.push((e.name.clone(), if e.is_dir { t("Folder") } else { t("File") }.to_string(), Act::Open(p, e.path.clone())));
        }
        let q = st.query.to_lowercase();
        let mut shown: Vec<usize> = (0..items.len()).filter(|&i| q.is_empty() || items[i].0.to_lowercase().contains(&q)).collect();
        shown.truncate(60);
        if st.sel >= shown.len() {
            st.sel = shown.len().saturating_sub(1);
        }
        let mut run: Option<usize> = None;
        let mut close = false;
        egui::Area::new(egui::Id::new("palette")).anchor(Align2::CENTER_TOP, [0.0, 70.0]).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.set_width(520.0);
                ui.horizontal(|ui| {
                    ui.label(ic(Ic::Search));
                    let r = ui.add(egui::TextEdit::singleline(&mut st.query).hint_text(t("Type a command or a folder name...")).desired_width(f32::INFINITY).frame(false));
                    if !st.focus_set {
                        r.request_focus();
                        st.focus_set = true;
                    }
                    if r.changed() {
                        st.sel = 0;
                    }
                });
                ui.separator();
                let (down, up, enter, esc) = ui.input(|i| {
                    (i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape))
                });
                if down && st.sel + 1 < shown.len() {
                    st.sel += 1;
                }
                if up && st.sel > 0 {
                    st.sel -= 1;
                }
                if enter && !shown.is_empty() {
                    run = Some(shown[st.sel]);
                }
                if esc {
                    close = true;
                }
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for (row, &i) in shown.iter().enumerate() {
                        let sel = row == st.sel;
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                        if sel {
                            ui.painter().rect_filled(r, CornerRadius::same(5), ui.visuals().selection.bg_fill.gamma_multiply(0.35));
                            ui.scroll_to_rect(r, None);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(5), ui.visuals().widgets.hovered.weak_bg_fill);
                        }
                        ui.painter().text(r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, &items[i].0, FontId::proportional(14.0), ui.visuals().text_color());
                        ui.painter().text(r.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, &items[i].1, FontId::proportional(11.0), ui.visuals().weak_text_color());
                        if resp.clicked() {
                            run = Some(i);
                        }
                    }
                });
            });
        });
        if let Some(i) = run {
            let (_, _, act) = items.swap_remove(i);
            self.acts.push(act);
            return;
        }
        if close || ctx.input(|i| i.pointer.any_pressed()) && !ctx.is_using_pointer() && false {
            return;
        }
        self.palette = Some(st);
    }
}

/* ----------------------------------------------------------------- treemap */

fn worst(row: &[f64], sum: f64, short: f64) -> f64 {
    let s2 = short * short;
    let mx = row.iter().cloned().fold(f64::MIN, f64::max);
    let mn = row.iter().cloned().fold(f64::MAX, f64::min);
    ((s2 * mx) / (sum * sum)).max((sum * sum) / (s2 * mn))
}

/// Squarified treemap layout.
pub fn squarify(values: &[f64], r: Rect) -> Vec<Rect> {
    let total: f64 = values.iter().sum();
    if total <= 0.0 || r.width() < 1.0 || r.height() < 1.0 {
        return Vec::new();
    }
    let area = r.width() as f64 * r.height() as f64;
    let scaled: Vec<f64> = values.iter().map(|v| v / total * area).collect();
    let mut out = Vec::with_capacity(values.len());
    let (mut x, mut y, mut w, mut h) = (r.min.x as f64, r.min.y as f64, r.width() as f64, r.height() as f64);
    let mut i = 0;
    while i < scaled.len() {
        let short = w.min(h).max(1.0);
        let mut end = i + 1;
        let mut sum = scaled[i];
        let mut best = worst(&scaled[i..end], sum, short);
        while end < scaled.len() {
            let ns = sum + scaled[end];
            let nw = worst(&scaled[i..=end], ns, short);
            if nw <= best {
                best = nw;
                sum = ns;
                end += 1;
            } else {
                break;
            }
        }
        if w >= h {
            let cw = (sum / h).max(0.0001);
            let mut cy = y;
            for k in i..end {
                let ch = scaled[k] / cw;
                out.push(Rect::from_min_size(pos2(x as f32, cy as f32), vec2(cw as f32, ch as f32)));
                cy += ch;
            }
            x += cw;
            w -= cw;
        } else {
            let rh = (sum / w).max(0.0001);
            let mut cx = x;
            for k in i..end {
                let cw = scaled[k] / rh;
                out.push(Rect::from_min_size(pos2(cx as f32, y as f32), vec2(cw as f32, rh as f32)));
                cx += cw;
            }
            y += rh;
            h -= rh;
        }
        i = end;
    }
    out
}

/// Draws the treemap; returns a folder to drill into when a tile was clicked.
fn draw_treemap(ui: &mut egui::Ui, rect: Rect, items: &[SizeItem]) -> Option<PathBuf> {
    let top: Vec<&SizeItem> = items.iter().take(80).collect();
    let vals: Vec<f64> = top.iter().map(|i| i.size as f64).collect();
    let rects = squarify(&vals, rect.shrink(1.0));
    let total: u64 = items.iter().map(|i| i.size).sum();
    let mut goto = None;
    let painter = ui.painter_at(rect);
    for (k, (it, r)) in top.iter().zip(rects.iter()).enumerate() {
        let base = if it.is_dir { kind_color(Kind::Folder) } else { kind_color(kind_of(&it.name.rsplit('.').next().unwrap_or("").to_lowercase())) };
        let hue = 0.78 + (k % 5) as f32 * 0.06;
        let col = Color32::from_rgb((base.r() as f32 * hue) as u8, (base.g() as f32 * hue) as u8, (base.b() as f32 * hue) as u8);
        let rr = r.shrink(1.0);
        let resp = ui.interact(rr, ui.id().with(("tm", k)), Sense::click());
        let col = if resp.hovered() { col.gamma_multiply(1.15) } else { col };
        painter.rect_filled(rr, CornerRadius::same(3), col);
        painter.rect_stroke(rr, CornerRadius::same(3), Stroke::new(1.0, Color32::from_black_alpha(60)), StrokeKind::Inside);
        if rr.width() > 54.0 && rr.height() > 30.0 {
            let pct = it.size as f64 / total.max(1) as f64 * 100.0;
            let g = painter.layout(it.name.clone(), FontId::proportional(12.5), Color32::WHITE, rr.width() - 8.0);
            painter.galley(rr.min + vec2(5.0, 4.0), g, Color32::WHITE);
            painter.text(rr.min + vec2(5.0, 20.0), Align2::LEFT_TOP, format!("{} · {:.1}%", fmt_size(it.size), pct), FontId::proportional(11.0), Color32::from_white_alpha(220));
        }
        let hover = format!("{}\n{}", it.path.display(), fmt_size(it.size));
        let resp = resp.on_hover_text(hover);
        if resp.clicked() && it.is_dir {
            goto = Some(it.path.clone());
        }
    }
    goto
}
