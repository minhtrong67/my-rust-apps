//! All app logic. Lives on the UI thread; workers (thumbnails, decoding, file jobs) post results back to it.
use crate::util::*;
use crate::{App, AppWindow, AlbumItem, DupGroup, DupItem, InfoRow, Row, Strip, Theme, Tile, Tr};
use rust_photos_core::{
    decode, exif_util, files,
    i18n::{self, Lang},
    library::{self, Entry, SortKey},
    store::{Album, FontSize, Material, Settings, ThemeMode, UserData, WindowMode},
    theme, thumbs,
};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedPixelBuffer, SharedString, Rgba8Pixel, VecModel, Weak};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};

thread_local! {
    static ST: RefCell<Option<State>> = const { RefCell::new(None) };
    static DEFER: RefCell<Vec<Box<dyn FnOnce(&mut State)>>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` with the app state. If a callback fires while the state is already in use, it is queued and run right after.
pub fn run(f: impl FnOnce(&mut State) + 'static) {
    let mut f = Some(f);
    ST.with(|c| match c.try_borrow_mut() {
        Ok(mut g) => {
            if let Some(s) = g.as_mut() {
                (f.take().unwrap())(s);
            }
        }
        Err(_) => DEFER.with(|d| d.borrow_mut().push(Box::new(f.take().unwrap()))),
    });
    loop {
        let next = DEFER.with(|d| {
            let mut d = d.borrow_mut();
            if d.is_empty() { None } else { Some(d.remove(0)) }
        });
        let Some(job) = next else { break };
        let ran = ST.with(|c| match c.try_borrow_mut() {
            Ok(mut g) => { if let Some(s) = g.as_mut() { job(s); } true }
            Err(_) => false,
        });
        if !ran { break; }
    }
}

/// Posts a closure to the UI thread that runs with the app state.
pub fn post(f: impl FnOnce(&mut State) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || run(f));
}

pub fn install(state: State) {
    ST.with(|c| *c.borrow_mut() = Some(state));
}

// ------------------------------------------------------------------ worker pool (newest job first)
type Job = Box<dyn FnOnce() + Send>;
pub struct Pool {
    q: Arc<(Mutex<Vec<Job>>, Condvar)>,
}
impl Pool {
    pub fn new(n: usize) -> Pool {
        let q: Arc<(Mutex<Vec<Job>>, Condvar)> = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        for _ in 0..n.max(1) {
            let q = q.clone();
            std::thread::spawn(move || loop {
                let job = {
                    let (m, cv) = &*q;
                    let mut g = m.lock().unwrap();
                    while g.is_empty() { g = cv.wait(g).unwrap(); }
                    g.pop().unwrap()
                };
                job();
            });
        }
        Pool { q }
    }
    pub fn submit(&self, j: Job) {
        let (m, cv) = &*self.q;
        m.lock().unwrap().push(j);
        cv.notify_one();
    }
}

#[derive(Clone, PartialEq)]
pub enum Scope { All, Fav, Albums, Album(String) }

/// A decoded picture ready for the viewer.
pub struct Prepared {
    pub buf: SharedPixelBuffer<Rgba8Pixel>,
    pub w: u32,
    pub h: u32,
    pub real: (u32, u32),
    pub exif: exif_util::ExifInfo,
    pub size: u64,
    pub mtime: u64,
    pub hist: [String; 4],
}

pub struct State {
    pub ui: Weak<AppWindow>,
    pub lang_cell: Rc<Cell<Lang>>,
    pub settings: Settings,
    pub data: UserData,
    pub pool: Pool,
    pub cache_dir: std::path::PathBuf,
    // library
    pub dir: String,
    pub is_library: bool,
    pub entries: Vec<Entry>,
    pub scope_entries: Vec<Entry>,
    pub scope: Scope,
    pub list: Vec<Entry>,
    pub folder_paths: Vec<String>,
    pub format_list: Vec<String>,
    pub search: String,
    pub selection: HashSet<String>,
    pub selecting: bool,
    pub grid_w: f32,
    pub cols: usize,
    pub gen: u64,
    pub row_models: Vec<Rc<VecModel<Tile>>>,
    pub tile_pos: Vec<(usize, usize)>,
    pub path_idx: HashMap<String, usize>,
    pub thumb_cache: HashMap<String, Image>,
    pub thumb_order: VecDeque<String>,
    // viewer
    pub current: Option<String>,
    pub ctx: Vec<String>,
    pub view_gen: u64,
    pub view_cache: HashMap<String, Arc<Prepared>>,
    pub view_order: VecDeque<String>,
    pub prepared: Option<Arc<Prepared>>,
    pub strip_model: Rc<VecModel<Strip>>,
    pub slide_timer: slint::Timer,
    // dialogs and toasts
    pub on_confirm: Option<Box<dyn FnOnce(&mut State)>>,
    pub on_prompt: Option<Box<dyn FnOnce(&mut State, String)>>,
    pub toast_timer: slint::Timer,
    pub toast_undo: Option<Vec<String>>,
    pub album_target: Vec<String>,
    pub dup_groups: Vec<Vec<String>>,
    pub dup_keep: Vec<usize>,
    pub batch_paths: Vec<String>,
    pub ed: Option<crate::editor::Editor>,
}

pub const GAP: f32 = 6.0;
pub const PAD: f32 = 36.0;

fn tile_default(idx: usize, e: &Entry, fav: bool, selected: bool, img: Option<Image>) -> Tile {
    Tile { idx: idx as i32, path: e.path.clone().into(), name: e.name.clone().into(), loaded: img.is_some(), thumb: img.unwrap_or_default(), fav, selected }
}

impl State {
    pub fn ui(&self) -> AppWindow {
        self.ui.upgrade().expect("window alive")
    }
    pub fn lang(&self) -> Lang { self.settings.lang }
    pub fn tr(&self, k: &str) -> String { i18n::tr(self.settings.lang, k) }
    pub fn trf(&self, k: &str, v: &[(&str, &str)]) -> String { i18n::trf(self.settings.lang, k, v) }

    // ------------------------------------------------------------------ toasts and dialogs
    pub fn toast(&mut self, kind: i32, msg: String, action: Option<(String, Vec<String>)>) {
        let ui = self.ui();
        let app = ui.global::<App>();
        // Full screen viewing: nothing may cover the picture, so only errors are shown.
        if app.get_fullscreen() && app.get_view() == 1 && kind != 3 { return; }
        app.set_toast_text(msg.into());
        app.set_toast_kind(kind);
        let has_action = action.is_some();
        app.set_toast_action(action.as_ref().map(|a| a.0.clone()).unwrap_or_default().into());
        self.toast_undo = action.map(|a| a.1);
        app.set_toast_visible(true);
        let weak = self.ui.clone();
        self.toast_timer.start(slint::TimerMode::SingleShot, std::time::Duration::from_millis(if has_action { 6500 } else { 3600 }), move || {
            if let Some(u) = weak.upgrade() { u.global::<App>().set_toast_visible(false); }
        });
    }
    pub fn error(&mut self, e: &str) {
        let m = if e.contains("unsupported") { self.tr("toast.unsupported") } else { self.trf("toast.error", &[("msg", e)]) };
        self.toast(if e.contains("unsupported") { 2 } else { 3 }, m, None);
    }

    pub fn confirm(&mut self, title: String, desc: String, yes: String, danger: bool, f: impl FnOnce(&mut State) + 'static) {
        let ui = self.ui(); let app = ui.global::<App>();
        app.set_confirm_title(title.into());
        app.set_confirm_desc(desc.into());
        app.set_confirm_yes(yes.into());
        app.set_confirm_danger(danger);
        app.set_confirm_open(true);
        self.on_confirm = Some(Box::new(f));
    }
    pub fn prompt(&mut self, title: String, label: String, value: String, yes: String, f: impl FnOnce(&mut State, String) + 'static) {
        let ui = self.ui(); let app = ui.global::<App>();
        app.set_prompt_title(title.into());
        app.set_prompt_label(label.into());
        app.set_prompt_value(value.into());
        app.set_prompt_yes(yes.into());
        app.set_prompt_open(true);
        self.on_prompt = Some(Box::new(f));
    }

    // ------------------------------------------------------------------ library
    pub fn load_library(&mut self) {
        let path = self.settings.library_path.clone();
        self.scope = Scope::All;
        let ui = self.ui(); let app = ui.global::<App>();
        app.set_scope(0);
        app.set_view(0);
        app.set_loading(true);
        self.is_library = true;
        std::thread::spawn(move || {
            let r = library::library(&path);
            post(move |s| s.library_loaded(r, true));
        });
    }

    pub fn open_path(&mut self, path: String) {
        self.ui().global::<App>().set_loading(true);
        self.is_library = false;
        std::thread::spawn(move || {
            let r = library::open_target(&path);
            post(move |s| s.library_loaded(r, false));
        });
    }

    fn library_loaded(&mut self, r: Result<library::Opened, String>, is_library: bool) {
        let ui = self.ui(); let app = ui.global::<App>();
        app.set_loading(false);
        app.set_loaded(true);
        match r {
            Err(e) => self.error(&e),
            Ok(o) => {
                if o.entries.is_empty() && !is_library {
                    let m = self.tr("toast.noImages");
                    self.toast(2, m, None);
                    return;
                }
                self.is_library = is_library;
                self.dir = o.dir;
                self.entries = o.entries;
                self.selection.clear();
                self.selecting = false;
                self.search.clear();
                app.set_search("".into());
                app.set_selecting(false);
                app.set_folder_index(0);
                app.set_format_index(0);
                self.scope = Scope::All;
                app.set_scope(0);
                if let Some(cur) = o.current {
                    let mut l = self.entries.clone();
                    library::sort_entries(&mut l, self.settings.sort);
                    let ctx: Vec<String> = l.into_iter().map(|e| e.path).collect();
                    self.rebuild();
                    self.open_viewer(cur, ctx);
                } else {
                    app.set_view(0);
                    self.rebuild();
                }
            }
        }
    }

    pub fn refresh(&mut self) {
        let dir = self.dir.clone();
        let lib = self.is_library;
        let libpath = self.settings.library_path.clone();
        if dir.is_empty() && !lib { return; }
        std::thread::spawn(move || {
            let r = if lib { library::library(&libpath) } else { library::open_target(&dir) };
            post(move |s| {
                if let Ok(o) = r {
                    s.dir = o.dir;
                    s.entries = o.entries;
                    s.load_scope_entries();
                    s.rebuild();
                }
            });
        });
    }

    pub fn load_scope_entries(&mut self) {
        let paths: Vec<String> = match &self.scope {
            Scope::Fav => self.data.favs.clone(),
            Scope::Album(id) => self.data.albums.iter().find(|a| &a.id == id).map(|a| a.paths.clone()).unwrap_or_default(),
            Scope::Albums => self.data.albums.iter().flat_map(|a| a.paths.iter().take(1).cloned()).collect(),
            Scope::All => { self.scope_entries.clear(); return; }
        };
        let mut seen = HashSet::new();
        let uniq: Vec<String> = paths.into_iter().filter(|p| seen.insert(p.clone())).collect();
        self.scope_entries = library::stat_paths(&uniq);
    }

    pub fn set_scope(&mut self, scope: Scope) {
        let ui = self.ui(); let app = ui.global::<App>();
        app.set_scope(match &scope { Scope::All => 0, Scope::Fav => 1, Scope::Albums => 2, Scope::Album(_) => 3 });
        self.scope = scope;
        app.set_view(0);
        self.selection.clear();
        self.selecting = false;
        app.set_selecting(false);
        app.set_folder_index(0);
        app.set_format_index(0);
        self.search.clear();
        app.set_search("".into());
        self.load_scope_entries();
        self.rebuild();
    }

    fn scope_title(&self) -> String {
        match &self.scope {
            Scope::All => if self.is_library { self.tr("gal.title") } else { crate::util::base_name(&self.dir) },
            Scope::Fav => self.tr("nav.favorites"),
            Scope::Albums => self.tr("nav.albums"),
            Scope::Album(id) => self.data.albums.iter().find(|a| &a.id == id).map(|a| a.name.clone()).unwrap_or_default(),
        }
    }

    /// Filters, sorts and groups the list, then rebuilds the rows the gallery shows.
    pub fn rebuild(&mut self) {
        let ui = self.ui();
        let app = ui.global::<App>();
        let lang = self.lang();
        let base: &Vec<Entry> = if self.scope == Scope::All { &self.entries } else { &self.scope_entries };

        // folder and format choices come from the whole base list
        let mut folders: Vec<String> = base.iter().map(|e| e.folder.clone()).collect::<HashSet<_>>().into_iter().collect();
        folders.sort();
        let mut formats: Vec<String> = base.iter().map(|e| ext_upper(&e.path)).collect::<HashSet<_>>().into_iter().collect();
        formats.sort();
        let fi = app.get_folder_index();
        let fo = app.get_format_index();
        let folder_sel = if fi > 0 { folders.get(fi as usize - 1).cloned() } else { None };
        let format_sel = if fo > 0 { formats.get(fo as usize - 1).cloned() } else { None };
        let q = self.search.to_lowercase();
        let mut list: Vec<Entry> = base
            .iter()
            .filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q))
            .filter(|e| folder_sel.as_ref().map_or(true, |f| &e.folder == f))
            .filter(|e| format_sel.as_ref().map_or(true, |f| &ext_upper(&e.path) == f))
            .cloned()
            .collect();
        if matches!(self.scope, Scope::Fav) && self.settings.sort == SortKey::DateDesc {
            // favourites keep the order they were added in unless the user picked another sort
            let order: HashMap<&String, usize> = self.data.favs.iter().enumerate().map(|(i, p)| (p, i)).collect();
            list.sort_by_key(|e| order.get(&e.path).copied().unwrap_or(usize::MAX));
        } else {
            library::sort_entries(&mut list, self.settings.sort);
        }

        let mut folder_items = vec![self.tr("gal.allFolders")];
        folder_items.extend(folders.iter().map(|f| base_name(f)));
        let mut format_items = vec![self.tr("gal.allFormats")];
        format_items.extend(formats.iter().cloned());
        self.folder_paths = folders;
        self.format_list = formats;
        app.set_folder_items(ModelRc::new(VecModel::from(folder_items.into_iter().map(SharedString::from).collect::<Vec<_>>())));
        app.set_format_items(ModelRc::new(VecModel::from(format_items.into_iter().map(SharedString::from).collect::<Vec<_>>())));

        // layout
        let tile = self.settings.tile as f32;
        let w = self.grid_w.max(240.0);
        self.cols = (((w - PAD + GAP) / (tile + GAP)).floor() as usize).max(1);
        app.set_tile_w(((w - PAD - GAP * (self.cols as f32 - 1.0)) / self.cols as f32).floor());

        let by_date = matches!(self.settings.sort, SortKey::DateDesc | SortKey::DateAsc) && self.scope != Scope::Fav;
        let grouped = self.settings.group && by_date;
        let today = chrono::Local::now().date_naive();
        let mut rows: Vec<Row> = Vec::new();
        self.row_models.clear();
        self.tile_pos = vec![(0, 0); list.len()];
        let mut i = 0usize;
        while i < list.len() {
            let mut end = (i + self.cols).min(list.len());
            if grouped {
                let label = library::group_label(list[i].mtime, lang, today);
                let mut j = i;
                while j < list.len() && library::group_label(list[j].mtime, lang, today) == label { j += 1; }
                rows.push(Row { head: true, title: label.into(), count: (j - i) as i32, tiles: ModelRc::default() });
                self.row_models.push(Rc::new(VecModel::default()));
                let mut k = i;
                while k < j {
                    end = (k + self.cols).min(j);
                    self.push_tile_row(&mut rows, &list, k, end);
                    k = end;
                }
                i = j;
            } else {
                self.push_tile_row(&mut rows, &list, i, end);
                i = end;
            }
        }
        self.path_idx = list.iter().enumerate().map(|(i, e)| (e.path.clone(), i)).collect();
        self.list = list;
        self.gen += 1;
        app.set_rows(ModelRc::new(VecModel::from(rows)));

        let n = self.list.len();
        app.set_count_text(if n == 1 { self.tr("photo.1") } else { self.trf("photos.n", &[("n", &n.to_string())]) }.into());
        app.set_scope_title(self.scope_title().into());
        let (t, d) = match &self.scope {
            Scope::Fav => (self.tr("fav.empty"), self.tr("fav.empty.desc")),
            Scope::Albums => (self.tr("alb.empty"), self.tr("alb.empty.desc")),
            Scope::Album(_) => (self.tr("alb.emptyAlbum"), String::new()),
            Scope::All => if !self.search.is_empty() || fi > 0 || fo > 0 { (self.tr("gal.noResults"), String::new()) } else { (self.tr("gal.empty.title"), self.tr("gal.empty.desc")) },
        };
        app.set_empty_title(t.into());
        app.set_empty_desc(d.into());
        self.update_albums_model();
        self.update_sel_text();
    }

    fn push_tile_row(&mut self, rows: &mut Vec<Row>, list: &[Entry], a: usize, b: usize) {
        let tiles: Vec<Tile> = (a..b)
            .map(|idx| {
                let e = &list[idx];
                tile_default(idx, e, self.data.is_fav(&e.path), self.selection.contains(&e.path), self.thumb_cache.get(&e.path).cloned())
            })
            .collect();
        let row_idx = self.row_models.len();
        for (c, idx) in (a..b).enumerate() { self.tile_pos[idx] = (row_idx, c); }
        let model = Rc::new(VecModel::from(tiles));
        rows.push(Row { head: false, title: SharedString::default(), count: 0, tiles: ModelRc::from(model.clone()) });
        self.row_models.push(model);
    }

    fn update_albums_model(&mut self) {
        let items: Vec<AlbumItem> = self
            .data
            .albums
            .iter()
            .map(|a| {
                let cover = a.paths.first().and_then(|p| self.thumb_cache.get(p).cloned());
                AlbumItem {
                    id: a.id.clone().into(),
                    name: a.name.clone().into(),
                    count: if a.paths.len() == 1 { self.tr("photo.1") } else { self.trf("photos.n", &[("n", &a.paths.len().to_string())]) }.into(),
                    has_cover: cover.is_some(),
                    cover: cover.unwrap_or_default(),
                }
            })
            .collect();
        // covers are loaded lazily the first time the albums page is shown
        let firsts: Vec<String> = self.data.albums.iter().filter_map(|a| a.paths.first().cloned()).collect();
        for p in firsts {
            if !self.thumb_cache.contains_key(&p) { self.request_thumb(0, usize::MAX, p); }
        }
        self.ui().global::<App>().set_albums(ModelRc::new(VecModel::from(items)));
    }

    fn update_sel_text(&mut self) {
        let n = self.selection.len();
        let ui = self.ui(); let app = ui.global::<App>();
        app.set_sel_count(n as i32);
        app.set_sel_text(self.trf("selected.n", &[("n", &n.to_string())]).into());
    }

    // ------------------------------------------------------------------ thumbnails
    pub fn remember_thumb(&mut self, path: String, img: Image) {
        if self.thumb_cache.insert(path.clone(), img).is_none() {
            self.thumb_order.push_back(path);
            // Each thumbnail is 256 KB of pixels (plus a GPU copy). Keep a few screens worth, drop the oldest.
            let cap = (self.cols * 30).max(400);
            while self.thumb_order.len() > cap {
                let Some(old) = self.thumb_order.pop_front() else { break };
                self.thumb_cache.remove(&old);
                if let Some(&i) = self.path_idx.get(&old) { self.clear_tile_image(i); }
            }
        }
    }

    fn clear_tile_image(&mut self, idx: usize) {
        if let Some(&(r, c)) = self.tile_pos.get(idx) {
            if let Some(m) = self.row_models.get(r) {
                if let Some(mut t) = m.row_data(c) {
                    t.thumb = Image::default();
                    t.loaded = false;
                    m.set_row_data(c, t);
                }
            }
        }
    }

    fn request_thumb(&mut self, gen: u64, idx: usize, path: String) {
        let dir = self.cache_dir.clone();
        self.pool.submit(Box::new(move || {
            if let Ok(img) = thumbs::load_thumb(&dir, Path::new(&path), thumbs::THUMB_SIZE) {
                let buf = to_buf(&img);
                post(move |s| s.thumb_ready(gen, idx, path, buf));
            }
        }));
    }

    pub fn need_thumb(&mut self, idx: usize) {
        let Some(e) = self.list.get(idx) else { return };
        let path = e.path.clone();
        if let Some(img) = self.thumb_cache.get(&path).cloned() {
            self.set_tile_image(idx, img);
        } else {
            let g = self.gen;
            self.request_thumb(g, idx, path);
        }
    }

    fn thumb_ready(&mut self, gen: u64, idx: usize, path: String, buf: SharedPixelBuffer<Rgba8Pixel>) {
        let img = Image::from_rgba8(buf);
        self.remember_thumb(path.clone(), img.clone());
        if gen == self.gen && self.list.get(idx).map(|e| e.path == path).unwrap_or(false) {
            self.set_tile_image(idx, img.clone());
        }
        // filmstrip and album covers use the same cache
        for i in 0..self.strip_model.row_count() {
            if let Some(mut st) = self.strip_model.row_data(i) {
                if self.ctx.get(st.idx as usize) == Some(&path) && !st.loaded {
                    st.thumb = img.clone();
                    st.loaded = true;
                    self.strip_model.set_row_data(i, st);
                }
            }
        }
        if self.scope == Scope::Albums { self.update_albums_model(); }
    }

    fn set_tile_image(&mut self, idx: usize, img: Image) {
        if let Some(&(r, c)) = self.tile_pos.get(idx) {
            if let Some(m) = self.row_models.get(r) {
                if let Some(mut t) = m.row_data(c) {
                    t.thumb = img;
                    t.loaded = true;
                    m.set_row_data(c, t);
                }
            }
        }
    }

    fn refresh_tile(&mut self, idx: usize) {
        let Some(e) = self.list.get(idx).cloned() else { return };
        if let Some(&(r, c)) = self.tile_pos.get(idx) {
            if let Some(m) = self.row_models.get(r) {
                if let Some(mut t) = m.row_data(c) {
                    t.fav = self.data.is_fav(&e.path);
                    t.selected = self.selection.contains(&e.path);
                    m.set_row_data(c, t);
                }
            }
        }
    }

    fn refresh_all_tiles(&mut self) {
        for i in 0..self.list.len() { self.refresh_tile(i); }
        self.update_sel_text();
    }

    pub fn forget_thumb(&mut self, path: &str) {
        self.thumb_cache.remove(path);
        self.thumb_order.retain(|p| p != path);
    }

    pub fn grid_resized(&mut self, w: f32) {
        if (w - self.grid_w).abs() < 0.5 { return; }
        self.grid_w = w;
        let new_cols = (((w.max(240.0) - PAD + GAP) / (self.settings.tile as f32 + GAP)).floor() as usize).max(1);
        if new_cols != self.cols && !self.list.is_empty() { self.rebuild(); } else {
            let ui = self.ui(); let app = ui.global::<App>();
            app.set_tile_w(((w.max(240.0) - PAD - GAP * (self.cols as f32 - 1.0)) / self.cols as f32).floor());
        }
    }
}

// ====================================================================== viewer
fn prepare(path: &str) -> Result<Prepared, String> {
    let p = Path::new(path);
    let img = decode::open_for_view(p, 6144)?;
    let meta = std::fs::metadata(p).ok();
    let o = exif_util::orientation(p);
    let real = image::image_dimensions(p).map(|(w, h)| if o >= 5 { (h, w) } else { (w, h) }).unwrap_or((img.width(), img.height()));
    let small = thumbs::resize_rgba(&img, 256);
    let h = rust_photos_core::edit::histogram(&small);
    let max = h.iter().flat_map(|c| c.iter()).copied().max().unwrap_or(1);
    Ok(Prepared {
        w: img.width(),
        h: img.height(),
        buf: to_buf(&img),
        real,
        exif: exif_util::read_info(p),
        size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
        mtime: meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0),
        hist: [hist_path(&h[3], max), hist_path(&h[0], max), hist_path(&h[1], max), hist_path(&h[2], max)],
    })
}

impl State {
    pub fn open_viewer(&mut self, path: String, ctx: Vec<String>) {
        self.ctx = ctx;
        self.current = Some(path);
        let ui = self.ui(); let app = ui.global::<App>();
        app.set_view(1);
        app.set_has_current(true);
        self.selecting = false;
        self.load_current();
    }

    fn pos(&self) -> usize {
        self.current.as_ref().and_then(|c| self.ctx.iter().position(|p| p == c)).unwrap_or(0)
    }

    pub fn load_current(&mut self) {
        let Some(path) = self.current.clone() else { return };
        let ui = self.ui(); let app = ui.global::<App>();
        let pos = self.pos();
        let n = self.ctx.len();
        app.set_view_name(base_name(&path).into());
        app.set_view_index_text(self.trf("view.of", &[("a", &(pos + 1).to_string()), ("b", &n.to_string())]).into());
        let looping = self.settings.slide.repeat && app.get_slideshow();
        app.set_has_prev(pos > 0 || (looping && n > 1));
        app.set_has_next(pos + 1 < n || (looping && n > 1));
        app.set_view_fav(self.data.is_fav(&path));
        self.update_strip(pos);
        self.view_gen += 1;
        let g = self.view_gen;
        if let Some(p) = self.view_cache.get(&path).cloned() {
            self.show_prepared(p);
        } else {
            app.set_view_loading(true);
            app.set_view_failed(false);
            let pth = path.clone();
            std::thread::spawn(move || {
                let r = prepare(&pth).map(Arc::new);
                post(move |s| s.view_decoded(g, pth, r));
            });
        }
        // decode the neighbours in the background so arrow keys feel instant
        for np in [pos.wrapping_sub(1), pos + 1] {
            if let Some(p) = self.ctx.get(np).cloned() {
                if !self.view_cache.contains_key(&p) {
                    std::thread::spawn(move || {
                        if let Ok(r) = prepare(&p) {
                            let pp = p.clone();
                            post(move |s| s.cache_prepared(pp, Arc::new(r)));
                        }
                    });
                }
            }
        }
    }

    fn cache_prepared(&mut self, path: String, p: Arc<Prepared>) {
        if self.view_cache.insert(path.clone(), p).is_none() {
            self.view_order.push_back(path);
            while self.view_order.len() > 3 {
                if let Some(old) = self.view_order.pop_front() { self.view_cache.remove(&old); }
            }
        }
    }

    fn view_decoded(&mut self, gen: u64, path: String, r: Result<Arc<Prepared>, String>) {
        match r {
            Ok(p) => {
                self.cache_prepared(path.clone(), p.clone());
                if gen == self.view_gen { self.show_prepared(p); }
            }
            Err(_) => {
                if gen == self.view_gen {
                    let ui = self.ui(); let app = ui.global::<App>();
                    app.set_view_loading(false);
                    app.set_view_failed(true);
                    app.set_view_w(0);
                }
            }
        }
    }

    fn show_prepared(&mut self, p: Arc<Prepared>) {
        let ui = self.ui(); let app = ui.global::<App>();
        app.set_view_image(Image::from_rgba8(p.buf.clone()));
        app.set_view_w(p.w as i32);
        app.set_view_h(p.h as i32);
        app.set_view_rev(app.get_view_rev() + 1);
        app.set_view_loading(false);
        app.set_view_failed(false);
        app.set_hist_luma(p.hist[0].clone().into());
        app.set_hist_r(p.hist[1].clone().into());
        app.set_hist_g(p.hist[2].clone().into());
        app.set_hist_b(p.hist[3].clone().into());
        self.prepared = Some(p);
        self.update_info();
    }

    fn update_info(&mut self) {
        let (Some(path), Some(p)) = (self.current.clone(), self.prepared.clone()) else { return };
        let mut rows: Vec<(String, String)> = Vec::new();
        let mut add = |s: &State, k: &str, v: String| { if !v.is_empty() { rows.push((s.tr(k), v)); } };
        add(self, "info.name", base_name(&path));
        add(self, "info.folder", dir_name(&path));
        add(self, "info.size", library::fmt_size(p.size));
        add(self, "info.dimensions", format!("{} × {}", p.real.0, p.real.1));
        add(self, "info.modified", fmt_time(p.mtime));
        let x = &p.exif;
        add(self, "info.taken", x.date.clone().unwrap_or_default());
        add(self, "info.camera", [x.make.clone(), x.model.clone()].into_iter().flatten().collect::<Vec<_>>().join(" "));
        add(self, "info.lens", x.lens.clone().unwrap_or_default());
        add(self, "info.iso", x.iso.clone().unwrap_or_default());
        add(self, "info.aperture", x.f_number.clone().unwrap_or_default());
        add(self, "info.shutter", x.exposure.clone().unwrap_or_default());
        add(self, "info.focal", x.focal.clone().unwrap_or_default());
        if let (Some(la), Some(lo)) = (x.lat, x.lon) { add(self, "info.gps", format!("{la:.5}, {lo:.5}")); }
        add(self, "info.software", x.software.clone().unwrap_or_default());
        let model: Vec<InfoRow> = rows.into_iter().map(|(l, v)| InfoRow { label: l.into(), value: v.into() }).collect();
        self.ui().global::<App>().set_info_rows(ModelRc::new(VecModel::from(model)));
    }

    fn update_strip(&mut self, pos: usize) {
        let a = pos.saturating_sub(12);
        let b = (pos + 13).min(self.ctx.len());
        let items: Vec<Strip> = (a..b)
            .map(|i| {
                let img = self.thumb_cache.get(&self.ctx[i]).cloned();
                Strip { idx: i as i32, current: i == pos, loaded: img.is_some(), thumb: img.unwrap_or_default() }
            })
            .collect();
        self.strip_model.set_vec(items);
        self.ui().global::<App>().set_strip(ModelRc::from(self.strip_model.clone()));
    }

    pub fn need_strip_thumb(&mut self, idx: usize) {
        if let Some(p) = self.ctx.get(idx).cloned() {
            if self.thumb_cache.contains_key(&p) { return; }
            self.request_thumb(u64::MAX, usize::MAX, p);
        }
    }

    pub fn step(&mut self, delta: i32) {
        let n = self.ctx.len() as i32;
        if n == 0 { return; }
        let looping = self.settings.slide.repeat && self.ui().global::<App>().get_slideshow();
        let mut t = self.pos() as i32 + delta;
        if t < 0 || t >= n {
            if looping { t = t.rem_euclid(n); } else { return; }
        }
        self.current = Some(self.ctx[t as usize].clone());
        self.load_current();
    }

    pub fn goto(&mut self, idx: usize) {
        if let Some(p) = self.ctx.get(idx).cloned() { self.current = Some(p); self.load_current(); }
    }

    pub fn toggle_slideshow(&mut self) {
        let ui = self.ui(); let app = ui.global::<App>();
        if app.get_slideshow() {
            self.slide_timer.stop();
            app.set_slideshow(false);
            return;
        }
        if self.ctx.len() < 2 { return; }
        app.set_slideshow(true);
        let secs = self.settings.slide.interval.max(1) as u64;
        self.slide_timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(secs), || post(|s| s.slide_tick()));
        self.load_current();
    }

    fn slide_tick(&mut self) {
        if self.settings.slide.shuffle {
            use rand::seq::SliceRandom;
            let cur = self.current.clone();
            let others: Vec<&String> = self.ctx.iter().filter(|p| Some(*p) != cur.as_ref()).collect();
            if let Some(p) = others.choose(&mut rand::thread_rng()) {
                self.current = Some((*p).clone());
                self.load_current();
            }
            return;
        }
        let at_end = self.pos() + 1 >= self.ctx.len();
        if at_end && !self.settings.slide.repeat {
            self.toggle_slideshow();
            return;
        }
        self.step(1);
    }

    pub fn back_to_gallery(&mut self) {
        let ui = self.ui(); let app = ui.global::<App>();
        if app.get_slideshow() { self.toggle_slideshow(); }
        app.set_fullscreen(false);
        app.set_view(0);
        self.view_cache.clear();
        self.view_order.clear();
        self.prepared = None;
        app.set_view_image(Image::default());
        app.set_view_w(0);
    }

    pub fn current_path(&self) -> Option<String> { self.current.clone() }

    pub fn path_at(&self, idx: i32) -> Option<String> {
        self.list.get(idx as usize).map(|e| e.path.clone())
    }

    // ------------------------------------------------------------------ file operations
    pub fn delete_paths(&mut self, paths: Vec<String>) {
        if paths.is_empty() { return; }
        if self.settings.confirm_delete {
            let one = paths.len() == 1;
            let (t, d) = if one {
                (self.tr("del.title"), self.trf("del.desc", &[("name", &base_name(&paths[0]))]))
            } else {
                (self.trf("del.title.n", &[("n", &paths.len().to_string())]), self.tr("del.desc.n"))
            };
            let y = self.tr("del.confirm");
            self.confirm(t, d, y, true, move |s| s.do_delete(paths));
        } else {
            self.do_delete(paths);
        }
    }

    fn do_delete(&mut self, paths: Vec<String>) {
        std::thread::spawn(move || {
            let mut done = Vec::new();
            let mut err = None;
            for p in paths {
                match files::delete_file(&p) { Ok(()) => done.push(p), Err(e) => err = Some(e) }
            }
            post(move |s| s.deleted(done, err));
        });
    }

    fn deleted(&mut self, done: Vec<String>, err: Option<String>) {
        if let Some(e) = err { self.error(&e); }
        if done.is_empty() { return; }
        let set: HashSet<&String> = done.iter().collect();
        self.entries.retain(|e| !set.contains(&e.path));
        self.scope_entries.retain(|e| !set.contains(&e.path));
        for p in &done { self.selection.remove(p); self.forget_thumb(p); self.view_cache.remove(p); }
        self.data.forget(&done);
        let _ = self.data.save();
        let was_current = self.current.as_ref().map(|c| set.contains(c)).unwrap_or(false);
        let pos = self.pos();
        self.ctx.retain(|p| !set.contains(p));
        self.rebuild();
        if self.ui().global::<App>().get_view() == 1 && was_current {
            if self.ctx.is_empty() {
                self.current = None;
                self.ui().global::<App>().set_has_current(false);
                self.back_to_gallery();
            } else {
                self.current = Some(self.ctx[pos.min(self.ctx.len() - 1)].clone());
                self.load_current();
            }
        }
        let msg = if done.len() == 1 { self.tr("toast.deleted") } else { self.trf("toast.deleted.n", &[("n", &done.len().to_string())]) };
        let undo = self.tr("undo");
        self.toast(1, msg, Some((undo, done)));
    }

    pub fn undo_delete(&mut self) {
        let Some(paths) = self.toast_undo.take() else { return };
        self.ui().global::<App>().set_toast_visible(false);
        std::thread::spawn(move || {
            let mut err = None;
            for p in &paths { if let Err(e) = files::restore_file(p) { err = Some(e); break; } }
            post(move |s| {
                match err {
                    Some(e) => s.error(&e),
                    None => { s.refresh(); let m = s.tr("toast.restored"); s.toast(0, m, None); }
                }
            });
        });
    }

    pub fn rotate(&mut self, path: String, deg: u32) {
        let q = self.settings.quality;
        std::thread::spawn(move || {
            let r = files::rotate_file(Path::new(&path), deg, q);
            post(move |s| match r {
                Ok(()) => s.file_changed(&path),
                Err(e) => s.error(&e),
            });
        });
    }

    /// A picture changed on disk (rotate, edit): drop stale caches and reload.
    pub fn file_changed(&mut self, path: &str) {
        self.forget_thumb(path);
        self.view_cache.remove(path);
        self.view_order.retain(|p| p != path);
        let fresh = library::stat_paths(&[path.to_string()]);
        if let Some(f) = fresh.first() {
            for l in [&mut self.entries, &mut self.scope_entries] {
                if let Some(e) = l.iter_mut().find(|e| e.path == path) { *e = f.clone(); }
            }
        }
        if self.current.as_deref() == Some(path) { self.load_current(); }
        self.rebuild();
    }

    pub fn rename_dialog(&mut self, path: String) {
        let (t, l, y) = (self.tr("rename.title"), self.tr("rename.label"), self.tr("rename"));
        let name = base_name(&path);
        self.prompt(t, l, name.clone(), y, move |s, new| {
            if new.is_empty() || new == name { return; }
            match files::rename_file(&path, &new) {
                Ok(np) => {
                    s.data.repath(&path, &np);
                    let _ = s.data.save();
                    for p in s.ctx.iter_mut().filter(|p| **p == path) { *p = np.clone(); }
                    if s.current.as_deref() == Some(path.as_str()) { s.current = Some(np.clone()); s.load_current(); }
                    s.refresh();
                }
                Err(_) => { let m = s.tr("toast.renameFail"); s.toast(3, m, None); }
            }
        });
    }

    pub fn transfer(&mut self, paths: Vec<String>, mv: bool) {
        slint::spawn_local(async move {
            let Some(h) = rfd::AsyncFileDialog::new().pick_folder().await else { return };
            let dest = h.path().to_string_lossy().into_owned();
            std::thread::spawn(move || {
                let r = files::transfer_files(&paths, &dest, mv);
                post(move |s| match r {
                    Ok(n) => {
                        let verb = s.tr(if mv { "xfer.move" } else { "xfer.copy" });
                        let m = s.trf("toast.transferred", &[("n", &n.to_string()), ("verb", &verb)]);
                        s.toast(1, m, None);
                        if mv { s.refresh(); }
                    }
                    Err(e) => s.error(&e),
                });
            });
        }).ok();
    }

    pub fn pick_open_folder(&mut self) {
        slint::spawn_local(async move {
            if let Some(h) = rfd::AsyncFileDialog::new().pick_folder().await {
                let p = h.path().to_string_lossy().into_owned();
                run(move |s| s.open_path(p));
            }
        }).ok();
    }

    pub fn pick_open_files(&mut self) {
        slint::spawn_local(async move {
            let f = rfd::AsyncFileDialog::new().add_filter("Images", &library::EXTS).pick_files().await;
            if let Some(list) = f {
                if let Some(first) = list.first() {
                    let p = first.path().to_string_lossy().into_owned();
                    run(move |s| s.open_path(p));
                }
            }
        }).ok();
    }

    pub fn pick_library_folder(&mut self) {
        slint::spawn_local(async move {
            if let Some(h) = rfd::AsyncFileDialog::new().pick_folder().await {
                let p = h.path().to_string_lossy().into_owned();
                run(move |s| { s.settings.library_path = p; s.save_settings(); s.sync_settings_ui(); s.load_library(); });
            }
        }).ok();
    }

    pub fn copy_image_of(&mut self, path: String) {
        std::thread::spawn(move || {
            let ok = decode::open_upright(Path::new(&path)).map(|i| copy_image(&i.to_rgba8())).unwrap_or(false);
            post(move |s| {
                let (k, m) = if ok { (1, s.tr("toast.copied")) } else { (3, s.tr("toast.copyFail")) };
                s.toast(k, m, None);
            });
        });
    }

    pub fn copy_path_of(&mut self, path: String) {
        let ok = copy_text(&path);
        let (k, m) = if ok { (1, self.tr("toast.pathCopied")) } else { (3, self.tr("toast.copyFail")) };
        self.toast(k, m, None);
    }

    pub fn wallpaper(&mut self, path: String) {
        let tmp = self.cache_dir.clone();
        std::thread::spawn(move || {
            let r = files::set_wallpaper(&path, &tmp);
            post(move |s| match r {
                Ok(()) => { let m = s.tr("toast.wallpaper"); s.toast(1, m, None); }
                Err(e) => s.error(&e),
            });
        });
    }

    // ------------------------------------------------------------------ favourites and albums
    pub fn toggle_fav(&mut self, path: &str) {
        self.data.toggle_fav(path);
        let _ = self.data.save();
        if self.scope == Scope::Fav { self.load_scope_entries(); self.rebuild(); }
        if let Some(i) = self.list.iter().position(|e| e.path == path) { self.refresh_tile(i); }
        if self.current.as_deref() == Some(path) { self.ui().global::<App>().set_view_fav(self.data.is_fav(path)); }
    }

    pub fn new_album(&mut self, paths: Vec<String>) {
        let (t, l, y) = (self.tr("alb.new"), self.tr("alb.name"), self.tr("create"));
        self.prompt(t, l, String::new(), y, move |s, name| {
            if name.is_empty() { return; }
            let id = format!("{:x}", rand::random::<u64>());
            s.data.albums.push(Album { id, name: name.clone(), paths: paths.clone() });
            let _ = s.data.save();
            if !paths.is_empty() { let m = s.trf("alb.added", &[("name", &name)]); s.toast(1, m, None); }
            s.update_albums_model();
        });
    }

    pub fn add_to_album(&mut self, id: &str, paths: Vec<String>) {
        let Some(a) = self.data.albums.iter_mut().find(|a| a.id == id) else { return };
        for p in paths { if !a.paths.contains(&p) { a.paths.push(p); } }
        let name = a.name.clone();
        let _ = self.data.save();
        let m = self.trf("alb.added", &[("name", &name)]);
        self.toast(1, m, None);
        self.update_albums_model();
    }

    pub fn open_album_picker(&mut self, paths: Vec<String>) {
        self.album_target = paths;
        self.update_albums_model();
        self.ui().global::<App>().set_album_pick_open(true);
    }

    // ------------------------------------------------------------------ batch rename, duplicates
    pub fn open_batch(&mut self, paths: Vec<String>) {
        self.batch_paths = paths;
        let ui = self.ui(); let app = ui.global::<App>();
        app.set_batch_open(true);
        self.update_batch();
    }

    pub fn update_batch(&mut self) {
        let ui = self.ui(); let app = ui.global::<App>();
        let pat = app.get_batch_pattern().to_string();
        let (start, digits) = (app.get_batch_start().max(0) as u32, app.get_batch_digits().max(1) as usize);
        let name_for = |i: usize, p: &str| {
            let b = base_name(p);
            let ext = Path::new(&b).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            format!("{}{}", files::build_name(&pat, &b, start + i as u32, digits), ext)
        };
        let prev: Vec<SharedString> = self.batch_paths.iter().take(6).enumerate().map(|(i, p)| format!("{}  →  {}", base_name(p), name_for(i, p)).into()).collect();
        app.set_batch_preview(ModelRc::new(VecModel::from(prev)));
        app.set_batch_apply_text(self.trf("br.apply", &[("n", &self.batch_paths.len().to_string())]).into());
    }

    pub fn apply_batch(&mut self) {
        let ui = self.ui(); let app = ui.global::<App>();
        let pat = app.get_batch_pattern().to_string();
        let (start, digits) = (app.get_batch_start().max(0) as u32, app.get_batch_digits().max(1) as usize);
        let mut n = 0;
        for (i, p) in self.batch_paths.clone().iter().enumerate() {
            let b = base_name(p);
            let ext = Path::new(&b).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            let name = format!("{}{}", files::build_name(&pat, &b, start + i as u32, digits), ext);
            if name == b { continue; }
            if let Ok(np) = files::rename_file(p, &name) { self.data.repath(p, &np); n += 1; }
        }
        let _ = self.data.save();
        app.set_batch_open(false);
        self.selection.clear();
        self.selecting = false;
        app.set_selecting(false);
        let m = self.trf("br.done", &[("n", &n.to_string())]);
        self.toast(1, m, None);
        self.refresh();
    }

    pub fn open_duplicates(&mut self) {
        let ui = self.ui(); let app = ui.global::<App>();
        app.set_dup_open(true);
        app.set_dup_scanning(true);
        app.set_dup_groups(ModelRc::default());
        let paths: Vec<String> = self.entries.iter().map(|e| e.path.clone()).collect();
        let dir = self.cache_dir.clone();
        std::thread::spawn(move || {
            let groups = files::find_duplicates(&paths);
            let shown: Vec<Vec<(String, String, u64, Option<SharedPixelBuffer<Rgba8Pixel>>)>> = groups
                .iter()
                .take(100)
                .map(|g| {
                    g.iter()
                        .map(|p| {
                            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                            let t = thumbs::load_thumb(&dir, Path::new(p), thumbs::THUMB_SIZE).ok().map(|i| to_buf(&i));
                            (p.clone(), base_name(p), size, t)
                        })
                        .collect()
                })
                .collect();
            post(move |s| s.duplicates_found(groups, shown));
        });
    }

    fn duplicates_found(&mut self, groups: Vec<Vec<String>>, shown: Vec<Vec<(String, String, u64, Option<SharedPixelBuffer<Rgba8Pixel>>)>>) {
        self.dup_groups = groups.into_iter().take(shown.len()).collect();
        self.dup_keep = vec![0; self.dup_groups.len()];
        let ui = self.ui(); let app = ui.global::<App>();
        app.set_dup_scanning(false);
        let mut spare = 0u64;
        let model: Vec<DupGroup> = shown
            .into_iter()
            .enumerate()
            .map(|(gi, g)| {
                let mut items = Vec::new();
                for (ii, (path, name, size, buf)) in g.into_iter().enumerate() {
                    if ii > 0 { spare += size; }
                    items.push(DupItem { path: path.into(), name: name.into(), size: library::fmt_size(size).into(), thumb: buf.map(Image::from_rgba8).unwrap_or_default(), keep: ii == 0 });
                }
                DupGroup { title: self.trf("dup.group", &[("n", &(gi + 1).to_string())]).into(), items: ModelRc::new(VecModel::from(items)) }
            })
            .collect();
        let n = model.len();
        app.set_dup_summary(if n == 0 { format!("{} — {}", self.tr("dup.none"), self.tr("dup.none.d")) } else { format!("{} · {}", self.trf("dup.found", &[("n", &n.to_string())]), self.trf("dup.recover", &[("size", &library::fmt_size(spare))])) }.into());
        let extra = self.dup_groups.iter().map(|g| g.len() - 1).sum::<usize>();
        app.set_dup_trash_text(self.tr("dup.trash").into());
        let _ = extra;
        app.set_dup_groups(ModelRc::new(VecModel::from(model)));
    }

    pub fn dup_toggle(&mut self, gi: usize, ii: usize) {
        if gi >= self.dup_keep.len() { return; }
        self.dup_keep[gi] = ii;
        let ui = self.ui(); let app = ui.global::<App>();
        let groups = app.get_dup_groups();
        if let Some(g) = groups.row_data(gi) {
            for k in 0..g.items.row_count() {
                if let Some(mut it) = g.items.row_data(k) { it.keep = k == ii; g.items.set_row_data(k, it); }
            }
        }
    }

    pub fn dup_trash(&mut self) {
        let mut extras: Vec<String> = Vec::new();
        for (gi, g) in self.dup_groups.iter().enumerate() {
            for (ii, p) in g.iter().enumerate() { if ii != self.dup_keep[gi] { extras.push(p.clone()); } }
        }
        if extras.is_empty() { return; }
        let (t, y) = (self.trf("dup.confirm", &[("n", &extras.len().to_string())]), self.tr("del.confirm"));
        self.confirm(t, String::new(), y, true, move |s| {
            s.ui().global::<App>().set_dup_open(false);
            s.do_delete(extras);
        });
    }
}

// ====================================================================== settings, look, actions
fn colors(c: &theme::Shades) -> ModelRc<slint::Color> {
    let f = |v: [u8; 3]| slint::Color::from_rgb_u8(v[0], v[1], v[2]);
    ModelRc::new(VecModel::from(vec![f(c.fill), f(c.hover), f(c.press), f(c.text), f(c.on)]))
}

impl State {
    pub fn save_settings(&mut self) {
        let _ = self.settings.save();
    }

    /// Theme, accent, text size and window material.
    pub fn apply_look(&mut self) {
        let ui = self.ui();
        let th = ui.global::<Theme>();
        th.set_mode(match self.settings.theme { ThemeMode::System => 0, ThemeMode::Light => 1, ThemeMode::Dark => 2 });
        th.set_acc_light(colors(&theme::accent_shades(&self.settings.accent, false)));
        th.set_acc_dark(colors(&theme::accent_shades(&self.settings.accent, true)));
        th.set_scale(match self.settings.font_size { FontSize::Sm => 0.875, FontSize::Md => 1.0, FontSize::Lg => 1.125 });
        th.set_reduce_motion(self.settings.reduce_motion);
        let dark = th.get_dark();
        let ok = crate::platform::apply_material(&ui, self.settings.material, dark);
        th.set_solid(self.settings.material == Material::None || !ok);
        ui.global::<App>().set_material_ok(ok);
    }

    pub fn sync_settings_ui(&mut self) {
        let ui = self.ui();
        let app = ui.global::<App>();
        let s = &self.settings;
        app.set_set_theme(match s.theme { ThemeMode::System => 0, ThemeMode::Light => 1, ThemeMode::Dark => 2 });
        app.set_set_accent_index(theme::ACCENTS.iter().position(|a| a.hex.eq_ignore_ascii_case(&s.accent)).map(|i| i as i32).unwrap_or(-1));
        app.set_set_material(match s.material { Material::None => 0, Material::Mica => 1, Material::MicaAlt => 2, Material::Acrylic => 3 });
        app.set_set_font_size(match s.font_size { FontSize::Sm => 0, FontSize::Md => 1, FontSize::Lg => 2 });
        app.set_set_lang(if s.lang == Lang::Vi { 0 } else { 1 });
        app.set_set_auto_load(s.auto_load);
        app.set_set_library_path(s.library_path.clone().into());
        app.set_set_confirm_delete(s.confirm_delete);
        app.set_set_keep_exif(s.keep_exif);
        app.set_set_quality(s.quality as i32);
        app.set_set_reduce_motion(s.reduce_motion);
        app.set_set_window_mode(if s.window_mode == WindowMode::Maximized { 1 } else { 0 });
        app.set_set_slide_interval(s.slide.interval as i32);
        app.set_set_slide_transition(match s.slide.transition { rust_photos_core::store::Transition::Fade => 0, rust_photos_core::store::Transition::Slide => 1, rust_photos_core::store::Transition::Zoom => 2 });
        app.set_set_slide_shuffle(s.slide.shuffle);
        app.set_set_slide_loop(s.slide.repeat);
        app.set_tile_size(s.tile as i32);
        app.set_group(s.group);
        app.set_nav_compact(s.nav_compact);
        app.set_sort_index(SortKey::ALL.iter().position(|k| *k == s.sort).unwrap_or(0) as i32);
        let sw: Vec<slint::Color> = theme::ACCENTS.iter().map(|a| { let c = theme::parse_hex(a.hex); slint::Color::from_rgb_u8(c[0], c[1], c[2]) }).collect();
        app.set_swatches(ModelRc::new(VecModel::from(sw)));
    }

    /// Strings that depend on the language: everything Rust sets as text.
    pub fn apply_lang(&mut self) {
        self.lang_cell.set(self.settings.lang);
        let ui = self.ui();
        ui.global::<Tr>().set_rev(ui.global::<Tr>().get_rev() + 1);
        let app = ui.global::<App>();
        let sorts: Vec<SharedString> = SortKey::ALL.iter().map(|k| self.tr(&format!("sort.{}", k.id())).into()).collect();
        app.set_sort_items(ModelRc::new(VecModel::from(sorts)));
        ui.set_title_text(self.tr("app").into());
        app.set_dup_trash_text(self.tr("dup.trash").into());
        self.rebuild();
        if self.prepared.is_some() { self.update_info(); }
    }

    pub fn setting(&mut self, key: &str, value: &str) {
        let i: i32 = value.parse().unwrap_or(0);
        let b = value == "1";
        let mut relayout = false;
        match key {
            "theme" => { self.settings.theme = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark][i.clamp(0, 2) as usize]; self.apply_look(); }
            "accent" => { if let Some(a) = theme::ACCENTS.get(i as usize) { self.settings.accent = a.hex.into(); self.apply_look(); } }
            "material" => { self.settings.material = [Material::None, Material::Mica, Material::MicaAlt, Material::Acrylic][i.clamp(0, 3) as usize]; self.apply_look(); }
            "font-size" => { self.settings.font_size = [FontSize::Sm, FontSize::Md, FontSize::Lg][i.clamp(0, 2) as usize]; self.apply_look(); }
            "lang" => { self.settings.lang = if i == 0 { Lang::Vi } else { Lang::En }; self.apply_lang(); }
            "reduce-motion" => { self.settings.reduce_motion = b; self.apply_look(); }
            "window-mode" => {
                self.settings.window_mode = if i == 1 { WindowMode::Maximized } else { WindowMode::Remember };
                if i == 1 { self.ui().window().set_maximized(true); }
            }
            "auto-load" => self.settings.auto_load = b,
            "confirm-delete" => self.settings.confirm_delete = b,
            "keep-exif" => self.settings.keep_exif = b,
            "quality" => self.settings.quality = i.clamp(50, 100) as u8,
            "slide-interval" => { self.settings.slide.interval = i.clamp(1, 60) as u32; if self.ui().global::<App>().get_slideshow() { self.toggle_slideshow(); self.toggle_slideshow(); } }
            "slide-transition" => self.settings.slide.transition = [rust_photos_core::store::Transition::Fade, rust_photos_core::store::Transition::Slide, rust_photos_core::store::Transition::Zoom][i.clamp(0, 2) as usize],
            "slide-shuffle" => self.settings.slide.shuffle = b,
            "slide-loop" => self.settings.slide.repeat = b,
            "sort" => { self.settings.sort = SortKey::ALL[i.clamp(0, 5) as usize]; relayout = true; }
            "tile" => { self.settings.tile = i.clamp(110, 320) as u32; self.ui().global::<App>().set_tile_size(i); relayout = true; }
            "group" => { self.settings.group = b; self.ui().global::<App>().set_group(b); relayout = true; }
            _ => {}
        }
        self.save_settings();
        if relayout { self.rebuild(); }
    }

    pub fn reset_settings(&mut self) {
        let keep_lang = self.settings.lang;
        self.settings = Settings::default();
        self.settings.lang = keep_lang;
        self.save_settings();
        self.sync_settings_ui();
        self.apply_look();
        self.apply_lang();
        let m = self.tr("set.resetDone");
        self.toast(1, m, None);
    }

    pub fn escape(&mut self) {
        let ui = self.ui(); let app = ui.global::<App>();
        if app.get_confirm_open() { app.set_confirm_open(false); self.on_confirm = None; }
        else if app.get_prompt_open() { app.set_prompt_open(false); self.on_prompt = None; }
        else if app.get_album_pick_open() { app.set_album_pick_open(false); }
        else if app.get_batch_open() { app.set_batch_open(false); }
        else if app.get_dup_open() { app.set_dup_open(false); }
        else if app.get_about_open() { app.set_about_open(false); }
        else if app.get_settings_open() { app.set_settings_open(false); }
        else if app.get_fullscreen() { app.set_fullscreen(false); }
        else if app.get_view() == 1 { self.back_to_gallery(); }
        else if app.get_view() == 0 && self.selecting { self.set_select_mode(false); }
    }

    fn set_select_mode(&mut self, on: bool) {
        self.selecting = on;
        if !on { self.selection.clear(); }
        self.ui().global::<App>().set_selecting(on);
        self.refresh_all_tiles();
    }

    fn toggle_select(&mut self, idx: usize) {
        let Some(e) = self.list.get(idx) else { return };
        let p = e.path.clone();
        if !self.selection.remove(&p) { self.selection.insert(p); }
        if !self.selection.is_empty() && !self.selecting {
            self.selecting = true;
            self.ui().global::<App>().set_selecting(true);
        }
        self.refresh_tile(idx);
        self.update_sel_text();
    }

    fn selected_paths(&self) -> Vec<String> {
        self.list.iter().filter(|e| self.selection.contains(&e.path)).map(|e| e.path.clone()).collect()
    }

    /// Every button, menu item and shortcut ends up here.
    pub fn act(&mut self, name: &str, i: i32) {
        let ui = self.ui();
        let app = ui.global::<App>();
        let tile_path = || self.path_at(i);
        match name {
            // ---- navigation
            "scope" => self.set_scope(match i { 0 => Scope::All, 1 => Scope::Fav, _ => Scope::Albums }),
            "nav-toggle" => { let v = !app.get_nav_compact(); app.set_nav_compact(v); self.settings.nav_compact = v; self.save_settings(); }
            "goto-viewer" => { if self.current.is_some() { app.set_view(1); self.load_current(); } }
            "esc" => self.escape(),
            "back" => self.back_to_gallery(),
            "fullscreen" => { let v = !app.get_fullscreen(); app.set_fullscreen(v); }
            // ---- gallery
            "open-folder" => self.pick_open_folder(),
            "open-files" => self.pick_open_files(),
            "refresh" => { if self.scope == Scope::All { self.refresh(); } else { self.load_scope_entries(); self.rebuild(); } let m = self.tr("toast.refreshed"); self.toast(0, m, None); }
            "filter" => self.rebuild(),
            "select-mode" => { let on = !self.selecting; self.set_select_mode(on); }
            "select-all" => { self.selecting = true; app.set_selecting(true); self.selection = self.list.iter().map(|e| e.path.clone()).collect(); self.refresh_all_tiles(); }
            "toggle" => self.toggle_select(i as usize),
            "open" => {
                if self.selecting { self.toggle_select(i as usize); } else if let Some(p) = tile_path() {
                    let ctx: Vec<String> = self.list.iter().map(|e| e.path.clone()).collect();
                    self.open_viewer(p, ctx);
                }
            }
            "fav" => { if let Some(p) = tile_path() { self.toggle_fav(&p); } }
            "edit" => { if let Some(p) = tile_path() { let ctx: Vec<String> = self.list.iter().map(|e| e.path.clone()).collect(); self.open_viewer(p, ctx); self.open_editor(); } }
            "album" => { if let Some(p) = tile_path() { self.open_album_picker(vec![p]); } }
            "copy-image" => { if let Some(p) = tile_path() { self.copy_image_of(p); } }
            "copy-path" => { if let Some(p) = tile_path() { self.copy_path_of(p); } }
            "reveal" => { if let Some(p) = tile_path() { reveal(&p); } }
            "wallpaper" => { if let Some(p) = tile_path() { self.wallpaper(p); } }
            "rename" => { if let Some(p) = tile_path() { self.rename_dialog(p); } }
            "copy-to" => { if let Some(p) = tile_path() { self.transfer(vec![p], false); } }
            "move-to" => { if let Some(p) = tile_path() { self.transfer(vec![p], true); } }
            "delete" => { if let Some(p) = tile_path() { self.delete_paths(vec![p]); } }
            // ---- selection bar
            "sel-fav" => { let paths = self.selected_paths(); let all = paths.iter().all(|p| self.data.is_fav(p)); for p in paths { if self.data.is_fav(&p) == all { self.data.toggle_fav(&p); } } let _ = self.data.save(); self.refresh_all_tiles(); }
            "sel-album" => { let p = self.selected_paths(); self.open_album_picker(p); }
            "sel-copy" => { let p = self.selected_paths(); self.transfer(p, false); }
            "sel-move" => { let p = self.selected_paths(); self.transfer(p, true); }
            "sel-rename" => { let p = self.selected_paths(); self.open_batch(p); }
            "sel-delete" => { let p = self.selected_paths(); self.delete_paths(p); }
            "batch-apply" => self.apply_batch(),
            // ---- albums
            "album-new" => self.new_album(vec![]),
            "album-new-pick" => { let t = std::mem::take(&mut self.album_target); app.set_album_pick_open(false); self.new_album(t); }
            "album-open" => { if let Some(a) = self.data.albums.get(i as usize) { let id = a.id.clone(); self.set_scope(Scope::Album(id)); } }
            "album-rename" => {
                if let Some(a) = self.data.albums.get(i as usize).cloned() {
                    let (t, l, y) = (self.tr("alb.rename"), self.tr("alb.name"), self.tr("rename"));
                    self.prompt(t, l, a.name, y, move |s, n| { if n.is_empty() { return; } if let Some(x) = s.data.albums.iter_mut().find(|x| x.id == a.id) { x.name = n; } let _ = s.data.save(); s.update_albums_model(); });
                }
            }
            "album-delete" => {
                if let Some(a) = self.data.albums.get(i as usize).cloned() {
                    let (t, d, y) = (self.tr("alb.delete"), self.trf("alb.deleteQ", &[("name", &a.name)]), self.tr("delete"));
                    self.confirm(t, d, y, true, move |s| { s.data.albums.retain(|x| x.id != a.id); let _ = s.data.save(); s.update_albums_model(); });
                }
            }
            // ---- viewer
            "prev" => self.step(-1),
            "next" => self.step(1),
            "goto" => self.goto(i as usize),
            "view-fav" => { if let Some(p) = self.current_path() { self.toggle_fav(&p); } }
            "view-edit" => self.open_editor(),
            "view-rotate" => { if let Some(p) = self.current_path() { self.rotate(p, i as u32); } }
            "slideshow" => self.toggle_slideshow(),
            "info" => {}
            "zoom-text" => app.set_zoom_text(format!("{i}%").into()),
            "view-copy" => { if let Some(p) = self.current_path() { self.copy_image_of(p); } }
            "view-copy-path" => { if let Some(p) = self.current_path() { self.copy_path_of(p); } }
            "view-reveal" => { if let Some(p) = self.current_path() { reveal(&p); } }
            "view-print" => { if let Some(p) = self.current_path() { if !print_image(&p) { let m = self.tr("toast.unsupported"); self.toast(2, m, None); } } }
            "view-wallpaper" => { if let Some(p) = self.current_path() { self.wallpaper(p); } }
            "view-rename" => { if let Some(p) = self.current_path() { self.rename_dialog(p); } }
            "view-copy-to" => { if let Some(p) = self.current_path() { self.transfer(vec![p], false); } }
            "view-move-to" => { if let Some(p) = self.current_path() { self.transfer(vec![p], true); } }
            "view-album" => { if let Some(p) = self.current_path() { self.open_album_picker(vec![p]); } }
            "view-delete" => { if let Some(p) = self.current_path() { self.delete_paths(vec![p]); } }
            "toast-action" => self.undo_delete(),
            // ---- dialogs and settings
            "duplicates" => self.open_duplicates(),
            "dup-trash" => self.dup_trash(),
            "lib-choose" => self.pick_library_folder(),
            "lib-reset" => { self.settings.library_path.clear(); self.save_settings(); self.sync_settings_ui(); self.load_library(); }
            "look-refresh" => self.apply_look(),
            "settings-reset" => self.reset_settings(),
            n if n.starts_with("ed-") => self.ed_act(n, i),
            _ => {}
        }
    }

    pub fn on_close(&mut self) -> bool {
        let ui = self.ui();
        let w = ui.window();
        let max = w.is_maximized();
        self.settings.window_max = max;
        if !max && !w.is_fullscreen() {
            let sz = w.size().to_logical(w.scale_factor());
            self.settings.window_w = sz.width.round() as u32;
            self.settings.window_h = sz.height.round() as u32;
        }
        self.save_settings();
        true
    }
}
