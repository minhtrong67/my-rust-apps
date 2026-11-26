//! Application state, background-job plumbing, navigation and file operations.

use crate::{
    dialogs::*,
    fsops::{self, *},
    i18n::t,
    model::*,
    settings::*,
};
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq, Debug)]
pub enum Loc {
    ThisPc,
    Dir(PathBuf),
}

impl Loc {
    pub fn dir(&self) -> Option<&Path> {
        match self {
            Loc::Dir(p) => Some(p),
            Loc::ThisPc => None,
        }
    }
    pub fn title(&self) -> String {
        match self {
            Loc::ThisPc => t("This PC").to_string(),
            Loc::Dir(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned()),
        }
    }
}

/* ------------------------------------------------------------------ messages */

pub enum Msg {
    Listed { tab: u64, gen: u64, res: Result<Vec<Entry>, String> },
    Searched { tab: u64, id: u64, res: Result<Vec<Entry>, String> },
    PasteDone { res: Result<Vec<(PathBuf, PathBuf)>, String>, mv: bool },
    Deleted { res: Result<(), String>, paths: Vec<PathBuf>, perm: bool },
    Changed(u64),
    Thumb { path: PathBuf, img: Option<Rgba> },
    BigImage { path: PathBuf, img: Option<Rgba> },
    Preview { path: PathBuf, res: Result<String, String> },
    Props { path: PathBuf, res: Result<Props, String> },
    Dupes(Vec<DupGroup>),
    Sizes { path: PathBuf, res: Result<Vec<SizeItem>, String> },
    DirSize { path: PathBuf, size: u64 },
    Cleaned(CleanResult),
    Archive { res: Result<PathBuf, String>, extracted: bool },
    Toast(ToastKind, String),
}

#[derive(Clone)]
pub struct Bus {
    pub tx: Sender<Msg>,
    pub ctx: egui::Context,
}
impl Bus {
    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint();
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}
pub struct Toast {
    pub kind: ToastKind,
    pub text: String,
    pub until: Instant,
}

/* -------------------------------------------------------------------- panes */

pub struct SearchState {
    pub query: String,
    pub id: u64,
    pub running: bool,
    pub results: Vec<Entry>,
}

pub struct RenameState {
    pub path: PathBuf,
    pub text: String,
    pub first: bool,
}

pub struct Tab {
    pub id: u64,
    pub loc: Loc,
    pub hist: Vec<Loc>,
    pub hpos: usize,
    pub entries: Vec<Entry>,
    pub view: Vec<usize>,
    pub sel: HashSet<PathBuf>,
    pub anchor: Option<usize>,
    pub filter: String,
    pub kind_filter: Option<Kind>,
    pub tag_filter: Option<u8>,
    pub mode: ViewMode,
    pub key: SortKey,
    pub asc: bool,
    pub loading: bool,
    pub gen: u64,
    pub error: Option<String>,
    pub search: Option<SearchState>,
    pub rename: Option<RenameState>,
    pub rename_after: Option<PathBuf>,
    pub select_after: Vec<PathBuf>,
    pub addr_edit: Option<String>,
    pub scroll_to: Option<PathBuf>,
    pub watcher: Option<notify::RecommendedWatcher>,
    pub dir_sizes: HashMap<PathBuf, u64>,
    pub total_size: u64,
    pub cols: usize,
}

impl Tab {
    pub fn new(id: u64, loc: Loc, s: &Settings) -> Tab {
        Tab {
            id,
            hist: vec![loc.clone()],
            loc,
            hpos: 0,
            entries: Vec::new(),
            view: Vec::new(),
            sel: HashSet::new(),
            anchor: None,
            filter: String::new(),
            kind_filter: None,
            tag_filter: None,
            mode: s.mode,
            key: s.sort_key,
            asc: s.sort_asc,
            loading: false,
            gen: 0,
            error: None,
            search: None,
            rename: None,
            rename_after: None,
            select_after: Vec::new(),
            addr_edit: None,
            scroll_to: None,
            watcher: None,
            dir_sizes: HashMap::new(),
            total_size: 0,
            cols: 1,
        }
    }

    pub fn list(&self) -> &Vec<Entry> {
        match &self.search {
            Some(s) => &s.results,
            None => &self.entries,
        }
    }

    pub fn rebuild(&mut self, s: &Settings) {
        let f = self.filter.to_lowercase();
        let list = self.list();
        let mut v: Vec<usize> = (0..list.len())
            .filter(|&i| {
                let e = &list[i];
                (s.show_hidden || !e.hidden)
                    && (f.is_empty() || e.name_lc.contains(&f))
                    && self.kind_filter.map_or(true, |k| e.kind() == k)
                    && self.tag_filter.map_or(true, |tg| s.tag_of(&e.path) == Some(tg))
            })
            .collect();
        let (key, asc) = (self.key, self.asc);
        let sizes = &self.dir_sizes;
        let size_of = |e: &Entry| if e.is_dir { sizes.get(&e.path).copied().unwrap_or(0) } else { e.size };
        v.sort_by(|&a, &b| {
            let (x, y) = (&list[a], &list[b]);
            y.is_dir.cmp(&x.is_dir).then_with(|| {
                let o = match key {
                    SortKey::Name => natural_cmp(&x.name_lc, &y.name_lc),
                    SortKey::Modified => x.modified.cmp(&y.modified),
                    SortKey::Kind => x.ext.cmp(&y.ext).then_with(|| natural_cmp(&x.name_lc, &y.name_lc)),
                    SortKey::Size => size_of(x).cmp(&size_of(y)),
                };
                if asc {
                    o
                } else {
                    o.reverse()
                }
            })
        });
        self.view = v;
    }

    pub fn selected_entries(&self) -> Vec<&Entry> {
        self.view.iter().map(|&i| &self.list()[i]).filter(|e| self.sel.contains(&e.path)).collect()
    }
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_entries().into_iter().map(|e| e.path.clone()).collect()
    }
    pub fn cur_dir(&self) -> Option<&Path> {
        self.loc.dir()
    }
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active: usize,
}
impl Pane {
    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
}

/* -------------------------------------------------------------------- misc */

pub struct Clip {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

pub struct Op {
    pub shared: Arc<Shared>,
    pub label: String,
}

pub enum Undo {
    Rename { from: PathBuf, to: PathBuf },
    Move { pairs: Vec<(PathBuf, PathBuf)> },
    Create { paths: Vec<PathBuf> },
    Trash { paths: Vec<PathBuf> },
}

#[derive(Default)]
pub struct Thumbs {
    pub tex: HashMap<PathBuf, TextureHandle>,
    pub pending: HashSet<PathBuf>,
    pub failed: HashSet<PathBuf>,
    pub order: VecDeque<PathBuf>,
}

impl Thumbs {
    pub fn request(&mut self, path: &Path, px: u32, bus: &Bus) {
        if self.tex.contains_key(path) || self.failed.contains(path) || self.pending.contains(path) || self.pending.len() >= 6 {
            return;
        }
        self.pending.insert(path.to_path_buf());
        let (p, b) = (path.to_path_buf(), bus.clone());
        std::thread::spawn(move || {
            let img = make_thumb(&p, px);
            b.send(Msg::Thumb { path: p, img });
        });
    }
}

pub struct QuickLook {
    pub path: PathBuf,
    pub text: Option<Result<String, String>>,
    pub tex: Option<TextureHandle>,
    pub loading: bool,
    pub zoom: f32,
    pub pan: egui::Vec2,
}

pub enum Act {
    Navigate(usize, Loc),
    Open(usize, PathBuf),
    NewTab(Loc),
    OpenOtherPane(PathBuf),
    NewWindow(Option<PathBuf>),
    Back(usize),
    Forward(usize),
    Up(usize),
    Refresh(usize),
    Copy(Vec<PathBuf>, bool),
    Paste(usize),
    Delete(Vec<PathBuf>, bool),
    Rename(usize, PathBuf),
    NewItem(usize, bool),
    Props(PathBuf),
    Batch(Vec<PathBuf>),
    Compress(Vec<PathBuf>),
    Extract(PathBuf, bool),
    OpenWith(&'static str, PathBuf),
    Reveal(PathBuf),
    CopyPath(Vec<PathBuf>),
    Pin(PathBuf, bool),
    Star(Vec<PathBuf>),
    Tag(Vec<PathBuf>, Option<u8>),
    FolderColor(PathBuf, Option<[u8; 3]>),
    Wallpaper(PathBuf),
    Custom(usize, PathBuf),
    Drop { paths: Vec<PathBuf>, dest: PathBuf, copy: bool },
    Focus(usize),
    QuickLook(PathBuf),
    Undo,
    CloseTab(usize, usize),
    SetTab(usize, usize),
    MoveTab(usize, usize, usize),
    Search(usize, String),
    ClearSearch(usize),
    SelectAll(usize),
    Sort(usize, SortKey),
    SetMode(usize, ViewMode),
    ToggleDual,
    Dialog(Dialog),
    System(&'static str),
    CleanTemp,
    Palette,
    CommitRename(usize),
    CancelRename(usize),
    FocusSearch,
    Usage(PathBuf),
    FocusAddress(usize),
}

pub struct App {
    pub s: Settings,
    pub panes: Vec<Pane>,
    pub focus: usize,
    pub tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
    pub ctx: egui::Context,
    pub clip: Option<Clip>,
    pub op: Option<Op>,
    pub undo: Vec<Undo>,
    pub toasts: Vec<Toast>,
    pub dialog: Dialog,
    pub palette: Option<PaletteState>,
    pub thumbs: Thumbs,
    pub drives: Vec<Drive>,
    pub drives_at: Instant,
    pub places: Vec<(&'static str, PathBuf)>,
    pub next_id: u64,
    pub search_seq: Arc<AtomicU64>,
    pub preview: Option<(PathBuf, Result<String, String>)>,
    pub preview_req: Option<PathBuf>,
    pub ql: Option<QuickLook>,
    pub acts: Vec<Act>,
    pub reload_at: HashMap<u64, Instant>,
    pub dirty: bool,
    pub saved_at: Instant,
    pub style_sig: Option<(bool, [u8; 3], u32)>,
    pub accent_checked: bool,
    pub fullscreen: bool,
    pub tools: Option<(bool, bool)>,
    pub dup_cancel: Arc<AtomicBool>,
    pub focus_search: bool,
}

impl App {
    pub fn new(ctx: egui::Context, start: Option<PathBuf>) -> App {
        let s = Settings::load();
        crate::i18n::set_vi(s.lang == "vi");
        let (tx, rx) = channel();
        let places = get_places();
        let mut app = App {
            panes: Vec::new(),
            focus: 0,
            tx,
            rx,
            ctx,
            clip: None,
            op: None,
            undo: Vec::new(),
            toasts: Vec::new(),
            dialog: Dialog::None,
            palette: None,
            thumbs: Thumbs::default(),
            drives: get_drives(),
            drives_at: Instant::now(),
            places,
            next_id: 1,
            search_seq: Arc::new(AtomicU64::new(0)),
            preview: None,
            preview_req: None,
            ql: None,
            acts: Vec::new(),
            reload_at: HashMap::new(),
            dirty: false,
            saved_at: Instant::now(),
            style_sig: None,
            accent_checked: false,
            fullscreen: false,
            tools: None,
            dup_cancel: Arc::new(AtomicBool::new(false)),
            focus_search: false,
            s,
        };
        // initial tabs
        let mut locs: Vec<Loc> = Vec::new();
        if let Some(p) = start.filter(|p| p.is_dir()) {
            locs.push(Loc::Dir(p));
        } else if app.s.restore_tabs {
            for t in app.s.tabs.clone() {
                let p = PathBuf::from(&t);
                if t == "::pc" {
                    locs.push(Loc::ThisPc);
                } else if p.is_dir() {
                    locs.push(Loc::Dir(p));
                }
            }
        }
        if locs.is_empty() {
            locs.push(if app.s.start_page {
                Loc::ThisPc
            } else {
                app.places.first().map(|(_, p)| Loc::Dir(p.clone())).unwrap_or(Loc::ThisPc)
            });
        }
        let mut tabs = Vec::new();
        for l in locs {
            let id = app.next_id;
            app.next_id += 1;
            tabs.push(Tab::new(id, l, &app.s));
        }
        app.panes.push(Pane { tabs, active: 0 });
        if app.s.dual {
            app.add_second_pane();
        }
        for p in 0..app.panes.len() {
            for a in 0..app.panes[p].tabs.len() {
                app.panes[p].active = a;
                let loc = app.panes[p].tabs[a].loc.clone();
                app.apply_folder_view(p, a, &loc);
                app.load_tab(p, a);
            }
            app.panes[p].active = 0;
        }
        app
    }

    pub fn bus(&self) -> Bus {
        Bus { tx: self.tx.clone(), ctx: self.ctx.clone() }
    }

    pub fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.toasts.push(Toast { kind, text: text.into(), until: Instant::now() + Duration::from_millis(3600) });
        self.ctx.request_repaint();
    }
    pub fn info(&mut self, text: &str) {
        self.toast(ToastKind::Info, text);
    }
    pub fn ok(&mut self, text: &str) {
        self.toast(ToastKind::Success, text);
    }
    pub fn err(&mut self, text: impl Into<String>) {
        self.toast(ToastKind::Error, text);
    }

    /// Turns an internal error code into a friendly (translated) message.
    pub fn friendly(e: &str) -> String {
        match e {
            "invalid-name" => t("Invalid name").into(),
            "exists" => t("A file with that name already exists").into(),
            "into-itself" => t("Cannot copy a folder into itself").into(),
            "no-extractor" => t("No extractor found for this archive").into(),
            "no-winrar" => t("WinRAR is not installed").into(),
            "not-found" => t("Not found").into(),
            "unsupported" => t("Unsupported on this system").into(),
            "bad-regex" => t("Invalid regular expression").into(),
            "cancelled" => t("Cancelled").into(),
            other => other.to_string(),
        }
    }

    pub fn add_second_pane(&mut self) {
        let loc = self.panes[0].tab().loc.clone();
        let id = self.next_id;
        self.next_id += 1;
        self.panes.push(Pane { tabs: vec![Tab::new(id, loc, &self.s)], active: 0 });
        let i = self.panes.len() - 1;
        self.load_tab(i, 0);
    }

    pub fn fp(&self) -> usize {
        self.focus.min(self.panes.len() - 1)
    }

    /* ------------------------------------------------------------ navigation */

    fn apply_folder_view(&mut self, p: usize, a: usize, loc: &Loc) {
        let (mode, key, asc) = (self.s.mode, self.s.sort_key, self.s.sort_asc);
        let fv = loc.dir().and_then(|d| self.s.per_folder.get(d.to_string_lossy().as_ref()).copied());
        let tab = &mut self.panes[p].tabs[a];
        match fv {
            Some(v) => {
                tab.mode = v.mode;
                tab.key = v.key;
                tab.asc = v.asc;
            }
            None => {
                tab.mode = mode;
                tab.key = key;
                tab.asc = asc;
            }
        }
    }

    /// Remember the current view / sort for this folder (like Explorer's "layout per folder").
    pub fn remember_folder_view(&mut self, p: usize) {
        let tab = self.panes[p].tab();
        if let Some(d) = tab.loc.dir() {
            let fv = FolderView { mode: tab.mode, key: tab.key, asc: tab.asc };
            self.s.per_folder.insert(d.to_string_lossy().into_owned(), fv);
            self.dirty = true;
        }
    }

    pub fn navigate(&mut self, p: usize, loc: Loc, push: bool) {
        let p = p.min(self.panes.len() - 1);
        let a = self.panes[p].active;
        if let Loc::Dir(d) = &loc {
            if !d.is_dir() {
                self.err(format!("{}: {}", t("Cannot open this folder"), d.display()));
                return;
            }
            self.s.push_recent(&d.to_string_lossy());
            self.dirty = true;
        }
        self.apply_folder_view(p, a, &loc);
        let tab = &mut self.panes[p].tabs[a];
        if push {
            tab.hist.truncate(tab.hpos + 1);
            if tab.hist.last() != Some(&loc) {
                tab.hist.push(loc.clone());
            }
            tab.hpos = tab.hist.len() - 1;
        }
        tab.loc = loc;
        tab.filter.clear();
        tab.search = None;
        tab.sel.clear();
        tab.anchor = None;
        tab.rename = None;
        tab.addr_edit = None;
        tab.kind_filter = None;
        tab.tag_filter = None;
        tab.dir_sizes.clear();
        self.load_tab(p, a);
    }

    pub fn history_move(&mut self, p: usize, delta: i32) {
        let tab = self.panes[p].tab();
        let np = tab.hpos as i32 + delta;
        if np < 0 || np as usize >= tab.hist.len() {
            return;
        }
        let loc = tab.hist[np as usize].clone();
        self.panes[p].tab_mut().hpos = np as usize;
        self.navigate(p, loc, false);
    }

    pub fn go_up(&mut self, p: usize) {
        let up = self.panes[p].tab().loc.dir().map(|d| d.parent().map(|x| x.to_path_buf()));
        match up {
            Some(Some(parent)) => {
                let from = self.panes[p].tab().loc.dir().map(|d| d.to_path_buf());
                self.navigate(p, Loc::Dir(parent), true);
                self.panes[p].tab_mut().scroll_to = from.clone();
                if let Some(f) = from {
                    self.panes[p].tab_mut().select_after = vec![f];
                }
            }
            Some(None) => self.navigate(p, Loc::ThisPc, true),
            None => {}
        }
    }

    pub fn load_tab(&mut self, p: usize, a: usize) {
        let bus = self.bus();
        let tab = &mut self.panes[p].tabs[a];
        tab.gen += 1;
        tab.watcher = None;
        match tab.loc.clone() {
            Loc::ThisPc => {
                tab.entries.clear();
                tab.view.clear();
                tab.loading = false;
                tab.error = None;
                self.drives = get_drives();
                self.drives_at = Instant::now();
            }
            Loc::Dir(path) => {
                tab.loading = true;
                tab.error = None;
                let (id, gen) = (tab.id, tab.gen);
                // watch for changes
                let wb = bus.clone();
                if let Ok(mut w) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                    if let Ok(ev) = res {
                        use notify::EventKind::*;
                        if matches!(ev.kind, Create(_) | Remove(_) | Modify(_)) {
                            wb.send(Msg::Changed(id));
                        }
                    }
                }) {
                    use notify::Watcher;
                    if w.watch(&path, notify::RecursiveMode::NonRecursive).is_ok() {
                        tab.watcher = Some(w);
                    }
                }
                std::thread::spawn(move || {
                    let res = list_dir(&path);
                    bus.send(Msg::Listed { tab: id, gen, res });
                });
            }
        }
    }

    pub fn reload_all(&mut self) {
        for p in 0..self.panes.len() {
            for a in 0..self.panes[p].tabs.len() {
                self.load_tab(p, a);
            }
        }
    }

    fn find_tab(&mut self, id: u64) -> Option<(usize, usize)> {
        for (p, pane) in self.panes.iter().enumerate() {
            for (a, t) in pane.tabs.iter().enumerate() {
                if t.id == id {
                    return Some((p, a));
                }
            }
        }
        None
    }

    pub fn new_tab(&mut self, p: usize, loc: Loc) {
        let id = self.next_id;
        self.next_id += 1;
        let tab = Tab::new(id, loc.clone(), &self.s);
        self.panes[p].tabs.push(tab);
        let a = self.panes[p].tabs.len() - 1;
        self.panes[p].active = a;
        self.apply_folder_view(p, a, &loc);
        if let Loc::Dir(d) = &loc {
            self.s.push_recent(&d.to_string_lossy());
        }
        self.load_tab(p, a);
    }

    pub fn close_tab(&mut self, p: usize, a: usize) {
        let pane = &mut self.panes[p];
        if pane.tabs.len() <= 1 {
            return;
        }
        pane.tabs.remove(a);
        if pane.active >= pane.tabs.len() {
            pane.active = pane.tabs.len() - 1;
        } else if a < pane.active {
            pane.active -= 1;
        }
    }

    /* -------------------------------------------------------------- messages */

    pub fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Listed { tab, gen, res } => {
                    let Some((p, a)) = self.find_tab(tab) else { continue };
                    let s = &self.s;
                    let t = &mut self.panes[p].tabs[a];
                    if t.gen != gen {
                        continue;
                    }
                    t.loading = false;
                    match res {
                        Ok(v) => {
                            t.entries = v;
                            t.error = None;
                            t.total_size = t.entries.iter().map(|e| e.size).sum();
                            t.rebuild(s);
                            let keep: HashSet<PathBuf> = t.entries.iter().map(|e| e.path.clone()).collect();
                            t.sel.retain(|p| keep.contains(p));
                            if !t.select_after.is_empty() {
                                t.sel = t.select_after.drain(..).filter(|p| keep.contains(p)).collect();
                                if let Some(f) = t.view.iter().map(|&i| &t.entries[i]).find(|e| t.sel.contains(&e.path)) {
                                    t.scroll_to = Some(f.path.clone());
                                }
                            }
                            if let Some(rp) = t.rename_after.take() {
                                if let Some(e) = t.entries.iter().find(|e| e.path == rp) {
                                    t.sel = [rp.clone()].into_iter().collect();
                                    t.rename = Some(RenameState { path: rp.clone(), text: e.name.clone(), first: true });
                                    t.scroll_to = Some(rp);
                                }
                            }
                            // optional folder sizes, computed in the background
                            if self.s.folder_sizes {
                                let dirs: Vec<PathBuf> = t.entries.iter().filter(|e| e.is_dir).map(|e| e.path.clone()).collect();
                                let bus = self.bus();
                                std::thread::spawn(move || {
                                    let _ = par_map(&dirs, |d| {
                                        let size = quick_dir_size(d);
                                        bus.send(Msg::DirSize { path: d.clone(), size });
                                    });
                                });
                            }
                        }
                        Err(e) => {
                            t.entries.clear();
                            t.view.clear();
                            t.error = Some(e);
                        }
                    }
                }
                Msg::Searched { tab, id, res } => {
                    let Some((p, a)) = self.find_tab(tab) else { continue };
                    let s = &self.s;
                    let t = &mut self.panes[p].tabs[a];
                    let Some(sr) = t.search.as_mut() else { continue };
                    if sr.id != id {
                        continue;
                    }
                    sr.running = false;
                    match res {
                        Ok(v) => sr.results = v,
                        Err(e) => {
                            if e != "cancelled" {
                                let msg = Self::friendly(&e);
                                t.search = None;
                                t.rebuild(s);
                                self.err(msg);
                                continue;
                            }
                        }
                    }
                    t.rebuild(s);
                }
                Msg::PasteDone { res, mv } => {
                    self.op = None;
                    match res {
                        Ok(pairs) => {
                            let n = pairs.len();
                            if mv {
                                self.undo.push(Undo::Move { pairs: pairs.clone() });
                            } else {
                                self.undo.push(Undo::Create { paths: pairs.iter().map(|(_, b)| b.clone()).collect() });
                            }
                            for p in 0..self.panes.len() {
                                let t = self.panes[p].tab_mut();
                                t.select_after = pairs.iter().map(|(_, b)| b.clone()).collect();
                            }
                            self.reload_all();
                            self.ok(&format!("{} - {} {}", if mv { t("Moved") } else { t("Copied") }, n, t("items")));
                        }
                        Err(e) => {
                            self.reload_all();
                            if e == "cancelled" {
                                self.info(t("Cancelled"));
                            } else {
                                self.err(Self::friendly(&e));
                            }
                        }
                    }
                }
                Msg::Deleted { res, paths, perm } => {
                    self.op = None;
                    match res {
                        Ok(()) => {
                            if !perm {
                                self.undo.push(Undo::Trash { paths: paths.clone() });
                            }
                            self.ok(&format!("{} - {} {}", if perm { t("Deleted") } else { t("Moved to the Recycle Bin") }, paths.len(), t("items")));
                        }
                        Err(e) => self.err(Self::friendly(&e)),
                    }
                    self.reload_all();
                }
                Msg::Changed(id) => {
                    if self.find_tab(id).is_some() {
                        self.reload_at.entry(id).or_insert_with(|| Instant::now() + Duration::from_millis(300));
                    }
                }
                Msg::Thumb { path, img } => {
                    self.thumbs.pending.remove(&path);
                    match img {
                        Some(i) => {
                            let ci = ColorImage::from_rgba_unmultiplied([i.w, i.h], &i.px);
                            let tex = ctx.load_texture(path.to_string_lossy(), ci, TextureOptions::LINEAR);
                            self.thumbs.tex.insert(path.clone(), tex);
                            self.thumbs.order.push_back(path);
                            while self.thumbs.order.len() > 400 {
                                if let Some(old) = self.thumbs.order.pop_front() {
                                    self.thumbs.tex.remove(&old);
                                }
                            }
                        }
                        None => {
                            self.thumbs.failed.insert(path);
                        }
                    }
                }
                Msg::BigImage { path, img } => {
                    if let Some(q) = self.ql.as_mut() {
                        if q.path == path {
                            q.loading = false;
                            if let Some(i) = img {
                                let ci = ColorImage::from_rgba_unmultiplied([i.w, i.h], &i.px);
                                q.tex = Some(ctx.load_texture("quicklook", ci, TextureOptions::LINEAR));
                            } else {
                                q.text = Some(Err("nopreview".into()));
                            }
                        }
                    }
                }
                Msg::Preview { path, res } => {
                    if self.ql.as_ref().map_or(false, |q| q.path == path) {
                        if let Some(q) = self.ql.as_mut() {
                            q.loading = false;
                            q.text = Some(res.clone());
                        }
                    }
                    if self.preview_req.as_ref() == Some(&path) {
                        self.preview = Some((path, res));
                    }
                }
                Msg::Props { path, res } => {
                    if let Dialog::Props { path: dp, props } = &mut self.dialog {
                        if *dp == path {
                            *props = res.ok();
                        }
                    }
                }
                Msg::Dupes(g) => {
                    if let Dialog::Dupes(d) = &mut self.dialog {
                        d.scanning = false;
                        d.groups = g;
                        d.picked.clear();
                    }
                }
                Msg::Sizes { path, res } => {
                    if let Dialog::Usage(u) = &mut self.dialog {
                        if u.path == path {
                            u.loading = false;
                            match res {
                                Ok(v) => u.items = v,
                                Err(e) => {
                                    u.items.clear();
                                    u.error = Some(e);
                                }
                            }
                        }
                    }
                }
                Msg::DirSize { path, size } => {
                    for pane in self.panes.iter_mut() {
                        for t in pane.tabs.iter_mut() {
                            if t.entries.iter().any(|e| e.path == path) {
                                t.dir_sizes.insert(path.clone(), size);
                            }
                        }
                    }
                }
                Msg::Cleaned(r) => {
                    self.ok(&format!("{}: {} ({} {})", t("Freed"), fmt_size(r.freed), r.files, t("files")));
                }
                Msg::Archive { res, extracted } => {
                    if self.op.as_ref().map_or(false, |o| o.label == t("Compress")) {
                        self.op = None;
                    }
                    match res {
                        Ok(_) => self.ok(if extracted { t("Extracted") } else { t("Archive created") }),
                        Err(e) => self.err(Self::friendly(&e)),
                    }
                    self.reload_all();
                }
                Msg::Toast(k, s) => self.toast(k, s),
            }
        }
    }

    /// Debounced file-watcher reloads, drive refresh and settings autosave.
    pub fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let due: Vec<u64> = self.reload_at.iter().filter(|(_, at)| **at <= now).map(|(id, _)| *id).collect();
        for id in due {
            self.reload_at.remove(&id);
            if let Some((p, a)) = self.find_tab(id) {
                if self.panes[p].tabs[a].rename.is_none() {
                    self.load_tab(p, a);
                }
            }
        }
        if let Some(next) = self.reload_at.values().min() {
            ctx.request_repaint_after(next.saturating_duration_since(now) + Duration::from_millis(5));
        }
        self.toasts.retain(|t| t.until > now);
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.dirty && self.saved_at.elapsed() > Duration::from_secs(3) {
            self.save_now();
        }
    }

    pub fn save_now(&mut self) {
        self.s.tabs = self.panes[0]
            .tabs
            .iter()
            .map(|t| match &t.loc {
                Loc::ThisPc => "::pc".to_string(),
                Loc::Dir(p) => p.to_string_lossy().into_owned(),
            })
            .collect();
        self.s.dual = self.panes.len() > 1;
        self.s.save();
        self.dirty = false;
        self.saved_at = Instant::now();
    }

    /* ------------------------------------------------------- file operations */

    pub fn start_paste(&mut self, sources: Vec<PathBuf>, dest: PathBuf, mv: bool, policy: Policy) {
        if self.op.is_some() {
            self.info("...");
            return;
        }
        let shared = Arc::new(Shared::default());
        self.op = Some(Op { shared: shared.clone(), label: if mv { t("Moving...").into() } else { t("Copying...").into() } });
        let bus = self.bus();
        std::thread::spawn(move || {
            let res = fsops::paste(&sources, &dest, mv, policy, &shared);
            bus.send(Msg::PasteDone { res, mv });
        });
    }

    pub fn paste_into(&mut self, p: usize) {
        let Some(dest) = self.panes[p].tab().cur_dir().map(|d| d.to_path_buf()) else { return };
        let Some(clip) = &self.clip else { return };
        let (sources, mv) = (clip.paths.clone(), clip.cut);
        self.paste_to(sources, dest, mv);
    }

    pub fn paste_to(&mut self, sources: Vec<PathBuf>, dest: PathBuf, mv: bool) {
        let names = check_conflicts(&sources, &dest);
        if names.is_empty() {
            self.start_paste(sources, dest, mv, Policy::Rename);
            if mv {
                self.clip = None;
            }
        } else {
            self.dialog = Dialog::Conflict { sources, dest, mv, names };
        }
    }

    pub fn start_delete(&mut self, paths: Vec<PathBuf>, perm: bool) {
        if self.op.is_some() {
            return;
        }
        let bus = self.bus();
        self.op = Some(Op { shared: Arc::new(Shared::default()), label: t("Delete").into() });
        std::thread::spawn(move || {
            let res = delete_items(&paths, perm);
            bus.send(Msg::Deleted { res, paths, perm });
        });
    }

    pub fn do_undo(&mut self) {
        let Some(u) = self.undo.pop() else {
            self.info(t("Nothing to undo"));
            return;
        };
        let res: Result<(), String> = match u {
            Undo::Rename { from, to } => fs_rename(&to, &from),
            Undo::Move { pairs } => {
                let mut r = Ok(());
                for (from, to) in pairs.iter().rev() {
                    if let Err(e) = fs_rename(to, from) {
                        r = Err(e);
                    }
                }
                r
            }
            Undo::Create { paths } => trash::delete_all(&paths).map_err(|e| e.to_string()),
            Undo::Trash { paths } => restore_trashed(&paths).map(|_| ()),
        };
        match res {
            Ok(()) => {
                self.ok(t("Undone"));
                self.reload_all();
            }
            Err(e) => self.err(Self::friendly(&e)),
        }
    }

    pub fn commit_rename(&mut self, p: usize) {
        let tab = self.panes[p].tab_mut();
        let Some(r) = tab.rename.take() else { return };
        let old_name = r.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if r.text.trim() == old_name {
            return;
        }
        match rename_item(&r.path, &r.text) {
            Ok(to) => {
                self.undo.push(Undo::Rename { from: r.path, to: to.clone() });
                let tab = self.panes[p].tab_mut();
                tab.select_after = vec![to];
                self.load_tab(p, self.panes[p].active);
            }
            Err(e) => self.err(Self::friendly(&e)),
        }
    }

    pub fn start_search(&mut self, p: usize, query: String, opts: SearchOpts) {
        let Some(root) = self.panes[p].tab().cur_dir().map(|d| d.to_path_buf()) else { return };
        let id = self.search_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let tid = self.panes[p].tab().id;
        self.panes[p].tab_mut().search = Some(SearchState { query: query.clone(), id, running: true, results: Vec::new() });
        self.panes[p].tab_mut().sel.clear();
        if !query.trim().is_empty() {
            self.s.search_history.retain(|q| *q != query);
            self.s.search_history.insert(0, query.clone());
            self.s.search_history.truncate(12);
            self.dirty = true;
        }
        let (bus, seq) = (self.bus(), self.search_seq.clone());
        std::thread::spawn(move || {
            let res = fsops::search(&root, &query, 5000, &opts, id, &seq);
            bus.send(Msg::Searched { tab: tid, id, res });
        });
        let s = &self.s;
        self.panes[p].tab_mut().rebuild(s);
    }

    pub fn request_preview(&mut self, path: &Path, max: usize) {
        if self.preview_req.as_deref() == Some(path) {
            return;
        }
        self.preview_req = Some(path.to_path_buf());
        self.preview = None;
        let (bus, p) = (self.bus(), path.to_path_buf());
        std::thread::spawn(move || {
            let res = preview_text(&p, max);
            bus.send(Msg::Preview { path: p, res });
        });
    }

    pub fn open_quick_look(&mut self, path: PathBuf) {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let mut q = QuickLook { path: path.clone(), text: None, tex: None, loading: false, zoom: 1.0, pan: egui::Vec2::ZERO };
        let bus = self.bus();
        if path.is_dir() {
            q.text = Some(Err("folder".into()));
        } else if is_image_ext(&ext) {
            q.loading = true;
            let p = path.clone();
            std::thread::spawn(move || {
                let img = load_image_capped(&p, 4096);
                bus.send(Msg::BigImage { path: p, img });
            });
        } else if is_text_ext(&ext) || ext == "markdown" {
            q.loading = true;
            let p = path.clone();
            std::thread::spawn(move || {
                let res = preview_text(&p, 256 * 1024);
                bus.send(Msg::Preview { path: p, res });
            });
        } else {
            q.text = Some(Err("nopreview".into()));
        }
        self.ql = Some(q);
    }
}

fn fs_rename(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::rename(from, to).map_err(|e| e.to_string())
}
