#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Rust Photos: native photo viewer and editor. No WebView2, no JavaScript.
slint::include_modules!();

mod editor;
mod platform;
mod state;
mod util;

use rust_photos_core::{
    i18n,
    library, store,
    store::{Settings, UserData},
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use state::{run, Pool, Scope, State};
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

// ---- memory diagnostics: counts live heap bytes so Task Manager's number can be split into
// "my Rust heap" and "everything else" (graphics driver, DLLs, GPU textures, thread stacks).
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() { let n = LIVE.fetch_add(l.size(), Relaxed) + l.size(); PEAK.fetch_max(n, Relaxed); }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, new) };
        if !q.is_null() {
            if new >= l.size() { let n = LIVE.fetch_add(new - l.size(), Relaxed) + new - l.size(); PEAK.fetch_max(n, Relaxed); }
            else { LIVE.fetch_sub(l.size() - new, Relaxed); }
        }
        q
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;
pub fn heap_mb() -> (usize, usize) { (LIVE.load(Relaxed) >> 20, PEAK.load(Relaxed) >> 20) }

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let startup = library::arg_target(&args);

    // One window only: a second launch hands its picture to the running one and exits.
    let instance = single_instance::SingleInstance::new("RustPhotos.minhtrong67.v3").ok();
    let request = store::config_dir().join("open-request.txt");
    if let Some(i) = &instance {
        if !i.is_single() {
            if let Some(p) = &startup {
                let _ = std::fs::create_dir_all(store::config_dir());
                let _ = std::fs::write(&request, p);
            }
            return;
        }
    }

    let settings = Settings::load();
    let ui = AppWindow::new().expect("create window");
    let lang_cell = Rc::new(Cell::new(settings.lang));

    // translations are looked up in Rust; `Tr.rev` makes every label re-read them after a language change
    {
        let lc = lang_cell.clone();
        ui.global::<Tr>().on_translate(move |key, _rev| i18n::tr(lc.get(), key.as_str()).into());
    }

    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let cache_dir = rust_photos_core::thumbs::cache_dir();
    {
        let d = cache_dir.clone();
        std::thread::spawn(move || rust_photos_core::thumbs::prune(&d));
    }
    let st = State {
        ui: ui.as_weak(),
        lang_cell,
        settings: settings.clone(),
        data: UserData::load(),
        pool: Pool::new(cores.saturating_sub(1).clamp(2, 4)),
        cache_dir,
        dir: String::new(),
        is_library: true,
        entries: vec![],
        scope_entries: vec![],
        scope: Scope::All,
        list: vec![],
        folder_paths: vec![],
        format_list: vec![],
        search: String::new(),
        selection: HashSet::new(),
        selecting: false,
        grid_w: 900.0,
        cols: 1,
        gen: 0,
        row_models: vec![],
        tile_pos: vec![],
        path_idx: HashMap::new(),
        thumb_cache: HashMap::new(),
        thumb_order: VecDeque::new(),
        current: None,
        ctx: vec![],
        view_gen: 0,
        view_cache: HashMap::new(),
        view_order: VecDeque::new(),
        prepared: None,
        strip_model: Rc::new(VecModel::default()),
        slide_timer: slint::Timer::default(),
        on_confirm: None,
        on_prompt: None,
        toast_timer: slint::Timer::default(),
        toast_undo: None,
        album_target: vec![],
        dup_groups: vec![],
        dup_keep: vec![],
        batch_paths: vec![],
        ed: None,
    };
    state::install(st);

    // ---- callbacks from the UI
    let app = ui.global::<App>();
    app.on_act(|n, i| { let n = n.to_string(); run(move |s| s.act(&n, i)); });
    app.on_set(|k, v| { let (k, v) = (k.to_string(), v.to_string()); run(move |s| s.setting(&k, &v)); });
    app.on_need_thumb(|i| run(move |s| s.need_thumb(i as usize)));
    app.on_need_strip_thumb(|i| run(move |s| s.need_strip_thumb(i as usize)));
    app.on_grid_resized(|w| run(move |s| s.grid_resized(w)));
    app.on_search_edited(|t| { let t = t.to_string(); run(move |s| { s.search = t; s.rebuild(); }); });
    app.on_confirm_result(|yes| run(move |s| {
        s.ui().global::<App>().set_confirm_open(false);
        if let Some(f) = s.on_confirm.take() { if yes { f(s); } }
    }));
    app.on_prompt_result(|yes, text| {
        let text = text.to_string();
        run(move |s| {
            s.ui().global::<App>().set_prompt_open(false);
            if let Some(f) = s.on_prompt.take() { if yes { f(s, text.trim().to_string()); } }
        })
    });
    app.on_ed_adj_changed(|i, v| run(move |s| s.ed_adj_changed(i as usize, v)));
    app.on_ed_adj_commit(|| run(|s| s.ed_commit()));
    app.on_ed_angle_changed(|v| run(move |s| s.ed_angle_changed(v)));
    app.on_ed_crop_changed(|x, y, w, h| run(move |s| s.ed_crop_changed(x, y, w, h)));
    app.on_ed_commit(|| run(|s| s.ed_commit()));
    app.on_ed_canvas(|k, x, y| run(move |s| s.ed_canvas(k, x, y)));
    app.on_ed_text_edited(|t| { let t = t.to_string(); run(move |s| s.ed_text_edited(t)); });
    app.on_ed_style_changed(|| run(|s| s.ed_style_changed()));
    app.on_batch_changed(|| run(|s| s.update_batch()));
    app.on_dup_toggle(|g, i| run(move |s| s.dup_toggle(g as usize, i as usize)));
    app.on_pick_album(|id| {
        let id = id.to_string();
        run(move |s| {
            s.ui().global::<App>().set_album_pick_open(false);
            let t = std::mem::take(&mut s.album_target);
            s.add_to_album(&id, t);
        })
    });
    ui.window().on_close_requested(|| {
        run(|s| { s.on_close(); });
        slint::CloseRequestResponse::HideWindow
    });
    app.set_app_version(format!("Rust Photos {}", env!("CARGO_PKG_VERSION")).into());

    // ---- window size
    {
        let w = ui.window();
        match settings.window_mode {
            store::WindowMode::Maximized => w.set_maximized(true),
            store::WindowMode::Remember => {
                w.set_size(slint::LogicalSize::new(settings.window_w.max(640) as f32, settings.window_h.max(460) as f32));
                if settings.window_max { w.set_maximized(true); }
            }
        }
    }

    ui.show().expect("show window");

    // dropped files: winit reports them, Slint does not
    {
        use slint::winit_030::{winit::event::WindowEvent, EventResult, WinitWindowAccessor};
        let weak = ui.as_weak();
        ui.window().on_winit_window_event(move |_w, e| {
            match e {
                WindowEvent::HoveredFile(_) => { if let Some(u) = weak.upgrade() { u.global::<App>().set_drop_active(true); } }
                WindowEvent::HoveredFileCancelled => { if let Some(u) = weak.upgrade() { u.global::<App>().set_drop_active(false); } }
                WindowEvent::DroppedFile(p) => {
                    if let Some(u) = weak.upgrade() { u.global::<App>().set_drop_active(false); }
                    let p = p.to_string_lossy().into_owned();
                    run(move |s| s.open_path(p));
                }
                _ => {}
            }
            EventResult::Propagate
        });
    }

    // forward requests from second launches
    let poll = slint::Timer::default();
    {
        let req = request.clone();
        poll.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(500), move || {
            if let Ok(p) = std::fs::read_to_string(&req) {
                let _ = std::fs::remove_file(&req);
                let p = p.trim().to_string();
                if !p.is_empty() {
                    run(move |s| {
                        let ui = s.ui();
                        ui.window().set_minimized(false);
                        s.open_path(p);
                    });
                }
            }
        });
    }

    let stats = slint::Timer::default();
    if std::env::var("FP_STATS").is_ok() {
        stats.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), || {
            run(|s| {
                let (live, peak) = heap_mb();
                let thumbs = s.thumb_cache.len();
                let views: usize = s.view_cache.values().map(|p| (p.w as usize * p.h as usize * 4) >> 20).sum();
                s.ui().set_title_text(format!("Rust Photos | Rust heap {live} MB (peak {peak}) | thumbs {thumbs} x 0.25 MB | viewer cache {views} MB").into());
            });
        });
    }
    run(|s| {
        s.sync_settings_ui();
        s.apply_look();
        s.apply_lang();
    });
    match startup {
        Some(p) => run(move |s| s.open_path(p)),
        None => run(|s| {
            if s.settings.auto_load { s.load_library(); } else { s.ui().global::<App>().set_loaded(true); }
        }),
    }
    ui.run().expect("run");
    drop(instance);
    let _ = ModelRc::<i32>::default().row_count();
}
