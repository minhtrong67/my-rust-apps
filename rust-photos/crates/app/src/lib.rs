//! Rust Photos: native photo viewer and editor. No WebView2, no JavaScript.
slint::include_modules!();

pub mod editor;
pub mod platform;
pub mod state;
pub mod util;

use rust_photos_core::{
    i18n,
    library, store,
    store::{Settings, UserData},
};
use slint::{ComponentHandle, Model, ModelRc};
use state::{run, State};
use std::cell::Cell;
use std::rc::Rc;

/// Starts the app. `heap_mb` reports (live, peak) Rust heap in MB for the FP_STATS title bar.
pub fn run_app(heap_mb: fn() -> (usize, usize)) {
    util::install_crash_log();
    let t0 = std::time::Instant::now();
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
        // old thumbnails are cleaned up well after start, so it never competes with the first screen of pictures
        let d = cache_dir.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(10));
            rust_photos_core::thumbs::prune(&d);
        });
    }
    let st = State::new(ui.as_weak(), lang_cell, settings.clone(), UserData::load(), cache_dir, cores.saturating_sub(1).clamp(2, 4), t0);
    state::install(st);

    // ---- callbacks from the UI
    let app = ui.global::<App>();
    app.on_act(|n, i| { let n = n.to_string(); run(move |s| s.act(&n, i)); });
    app.on_set(|k, v| { let (k, v) = (k.to_string(), v.to_string()); run(move |s| s.setting(&k, &v)); });
    app.on_need_thumb(|i| run(move |s| s.need_thumb(i as usize)));
    app.on_need_strip_thumb(|i| run(move |s| s.need_strip_thumb(i as usize)));
    app.on_grid_resized(|w| run(move |s| s.grid_resized(w)));
    app.on_search_edited(|t| {
        let t = t.to_string();
        // wait for a short pause in typing: filtering a big library on every key press would stutter
        run(move |s| {
            s.search = t;
            s.search_timer.start(slint::TimerMode::SingleShot, std::time::Duration::from_millis(140), || run(|s| s.rebuild_top()));
        });
    });
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
    let ms_window = t0.elapsed().as_secs_f64() * 1000.0;

    // dropped files: winit reports them, Slint does not
    {
        use slint::winit_030::{winit::event::WindowEvent, EventResult, WinitWindowAccessor};
        let weak = ui.as_weak();
        ui.window().on_winit_window_event(move |_w, e| {
            match e {
                WindowEvent::HoveredFile(_) => { if let Some(u) = weak.upgrade() { u.global::<App>().set_drop_active(true); } }
                WindowEvent::Focused(true) => { run(|s| s.refresh_if_changed()); }
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
        poll.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(1000), move || {
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
        stats.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || {
            run(move |s| {
                let (live, peak) = heap_mb();
                let thumbs = s.thumb_cache.len();
                let views: usize = s.view_cache.values().map(|p| (p.w as usize * p.h as usize * 4) >> 20).sum();
                s.ui().set_title_text(format!("Rust Photos | heap {live} MB (peak {peak}) | thumbs {thumbs} | viewer {views} MB | startup: window {:.0} ms, scan {:.0} ms, filter {:.0} ms, rows {:.0} ms ({} pics)", s.ms_window, s.ms_scan, s.ms_filter, s.ms_rows, s.list.len()).into());
            });
        });
    }
    run(move |s| {
        s.ms_window = ms_window;
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
