//! Times the UI-thread work of the gallery with a synthetic library, using the real app code and Slint's
//! software platform (no window). Run with:  CARGO_PROFILE_DEV_OPT_LEVEL=2 cargo run -p rust-photos --example startbench
use rust_photos::state::{self, run};
use rust_photos::{App, AppWindow};
use rust_photos_core::{i18n::Lang, library::Entry, store::{Settings, UserData}};
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::ComponentHandle;
use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Instant;

struct P(Rc<MinimalSoftwareWindow>);
impl Platform for P {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> { Ok(self.0.clone()) }
}

fn synth(n: usize) -> Vec<Entry> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let mut seed = 99u64;
    (0..n).map(|i| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let age_days = (seed >> 33) % 900;
        Entry { path: format!("/photos/trip{:03}/IMG_{:06}.jpg", i % 200, i), name: format!("IMG_{:06}.jpg", i), size: 3_000_000 + (seed >> 40), mtime: now - age_days * 86_400_000 - (seed >> 50), folder: format!("/photos/trip{:03}", i % 200) }
    }).collect()
}

// ---- the grouping work as it was before (label and allocation for every picture) and as it is now
fn old_pass(list: &[Entry], lang: Lang) -> usize {
    use rust_photos_core::library::group_label;
    let today = chrono::Local::now().date_naive();
    let folders: HashSet<String> = list.iter().map(|e| e.folder.clone()).collect();
    let formats: HashSet<String> = list.iter().map(|e| e.path.rsplit('.').next().unwrap_or("").to_uppercase()).collect();
    let (mut i, mut groups) = (0, 0);
    while i < list.len() {
        let label = group_label(list[i].mtime, lang, today);
        let mut j = i;
        while j < list.len() && group_label(list[j].mtime, lang, today) == label { j += 1; }
        groups += 1;
        i = j;
    }
    groups + folders.len() + formats.len()
}
fn new_pass(list: &[Entry], lang: Lang) -> usize {
    use rust_photos_core::library::{group_label_date, local_date};
    let today = chrono::Local::now().date_naive();
    let mut folders: HashSet<&str> = HashSet::new();
    let mut formats: Vec<String> = Vec::new();
    for e in list { folders.insert(e.folder.as_str()); let ext = e.path.rsplit('.').next().unwrap_or(""); if !formats.iter().any(|f| f.eq_ignore_ascii_case(ext)) { formats.push(ext.to_ascii_uppercase()); } }
    let mut memo: Option<(chrono::NaiveDate, String)> = None;
    let mut label_of = |ms: u64| -> String { let d = local_date(ms).unwrap_or(today); if let Some((md, ml)) = &memo { if *md == d { return ml.clone(); } } let l = group_label_date(d, lang, today); memo = Some((d, l.clone())); l };
    let (mut i, mut groups) = (0, 0);
    while i < list.len() {
        let label = label_of(list[i].mtime);
        let mut j = i + 1;
        while j < list.len() && label_of(list[j].mtime) == label { j += 1; }
        groups += 1;
        i = j;
    }
    groups + folders.len() + formats.len()
}

fn ms<F: FnOnce() -> R, R>(f: F) -> (f64, R) { let t = Instant::now(); let r = f(); (t.elapsed().as_secs_f64() * 1000.0, r) }

fn main() {
    let win = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(P(win.clone()))).unwrap();
    let (t_new, ui) = ms(|| AppWindow::new().unwrap());
    win.set_size(slint::PhysicalSize::new(1280, 820));
    let (t_show, _) = ms(|| ui.show().unwrap());
    let (t_frame, _) = ms(|| { slint::platform::update_timers_and_animations(); let mut buf = vec![PremultipliedRgbaColor::default(); 1280 * 820]; win.draw_if_needed(|r| { r.render(&mut buf, 1280); }); });
    println!("window object created: {t_new:.1} ms | show: {t_show:.1} ms | first frame (SOFTWARE renderer, empty gallery): {t_frame:.1} ms");

    let cache = std::env::temp_dir().join("startbench-cache");
    let st = state::State::new(ui.as_weak(), Rc::new(Cell::new(Lang::En)), Settings::default(), UserData::default(), cache, 2, Instant::now());
    state::install(st);
    println!("\n{:>8} | {:>26} | {:>22} | {:>20} | {:>20}", "pictures", "first load: filter+sort / rows", "window narrower (rows)", "search (filter+rows)", "old vs new grouping");
    for n in [5_000usize, 20_000, 50_000] {
        let list = synth(n);
        run(move |s| { s.entries = list; s.grid_w = 1200.0; });
        let out = Rc::new(Cell::new((0.0f64, 0.0f64))); let o2 = out.clone();
        let (t_all, _) = ms(|| run(move |s| { s.rebuild_top(); o2.set((s.ms_filter, s.ms_rows)); }));
        let (f, r) = out.get();
        let (t_res, _) = ms(|| run(|s| s.grid_resized(820.0)));
        let (t_q, _) = ms(|| run(|s| { s.search = "IMG_0001".into(); s.rebuild_top(); }));
        run(|s| { s.search.clear(); s.grid_w = 1200.0; });
        let sample = synth(n);
        let (t_old, a) = ms(|| old_pass(&sample, Lang::En));
        let (t_new_pass, b) = ms(|| new_pass(&sample, Lang::En));
        assert_eq!(a, b, "old and new grouping must agree");
        println!("{n:>8} | {t_all:>7.0} ms ({f:>3.0} / {r:>3.0})      | {t_res:>15.0} ms     | {t_q:>13.0} ms     | {t_old:>5.0} -> {t_new_pass:>3.0} ms");
    }
    let _ = ui.global::<App>();
}
