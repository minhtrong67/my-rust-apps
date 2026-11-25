//! Renders the UI with Slint's software renderer into PNG files, so the look can be checked without a display.
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use std::rc::Rc;

use rust_photos::*;

struct P(Rc<MinimalSoftwareWindow>);
impl Platform for P {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> { Ok(self.0.clone()) }
}

fn pic(seed: u32, w: u32, h: u32) -> Image {
    let mut b = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    let px = b.make_mut_slice();
    for y in 0..h { for x in 0..w {
        let t = (x as f32 / w as f32, y as f32 / h as f32);
        let a = (seed * 53 % 255) as f32;
        px[(y * w + x) as usize] = Rgba8Pixel { r: (a * (1.0 - t.0) + 60.0 * t.1) as u8, g: (90.0 + 120.0 * t.1 * (1.0 - t.0)) as u8, b: (255.0 * t.0 * (0.4 + 0.6 * ((seed % 5) as f32 / 5.0))) as u8, a: 255 };
    } }
    Image::from_rgba8(b)
}

fn save(win: &Rc<MinimalSoftwareWindow>, w: u32, h: u32, name: &str) {
    slint::platform::update_timers_and_animations();
    let mut buf = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
    win.draw_if_needed(|r| { r.render(&mut buf, w as usize); });
    let mut img = image::RgbaImage::new(w, h);
    for (p, s) in img.pixels_mut().zip(&buf) { *p = image::Rgba([s.red, s.green, s.blue, 255]); }
    img.save(format!("/tmp/shots/{name}.png")).unwrap();
}

fn main() {
    std::fs::create_dir_all("/tmp/shots").unwrap();
    let win = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(P(win.clone()))).unwrap();
    let ui = AppWindow::new().unwrap();
    let (w, h) = (1280u32, 820u32);
    win.set_size(slint::PhysicalSize::new(w, h));
    ui.show().unwrap();
    let en: std::collections::HashMap<String, String> = serde_json::from_str(include_str!("../../core/i18n/en.json")).unwrap();
    ui.global::<Tr>().on_translate(move |k, _| en.get(k.as_str()).cloned().unwrap_or(k.to_string()).into());
    let th = ui.global::<Theme>();
    th.set_solid(true);
    let app = ui.global::<App>();
    let names = ["IMG_2041.jpg", "Beach.png", "Dalat trip 12.jpg", "DSC00918.jpg", "sunset.webp", "family.jpg", "Hoi An.jpg", "notes.png"];
    let mk_row = |start: usize, n: usize| {
        let tiles: Vec<Tile> = (0..n).map(|i| Tile { idx: (start + i) as i32, path: "".into(), name: names[(start + i) % 8].into(), thumb: pic((start + i) as u32 * 7 + 3, 200, 200), loaded: (start + i) % 9 != 8, fav: (start + i) % 5 == 1, selected: start + i == 2 }).collect();
        Row { head: false, title: "".into(), count: 0, tiles: ModelRc::new(VecModel::from(tiles)) }
    };
    let head = |t: &str, c: i32| Row { head: true, title: t.into(), count: c, tiles: ModelRc::default() };
    app.set_rows(ModelRc::new(VecModel::from(vec![head("Today", 11), mk_row(0, 5), mk_row(5, 5), head("Yesterday", 5), mk_row(10, 5), head("November 2026", 3), mk_row(15, 3)])));
    app.set_tile_w(232.0);
    app.set_scope_title("Pictures".into());
    app.set_count_text("19 photos".into());
    app.set_loaded(true);
    app.set_folder_items(ModelRc::new(VecModel::from(vec![SharedString::from("All folders"), "Trips".into()])));
    app.set_format_items(ModelRc::new(VecModel::from(vec![SharedString::from("All formats")])));
    app.set_sort_items(ModelRc::new(VecModel::from(vec![SharedString::from("Date: newest first")])));
    app.set_sort_index(0);
    app.set_has_current(true);
    app.set_tile_size(190);
    let args: Vec<String> = std::env::args().collect();
    let dark = args.get(1).map(|a| a == "dark").unwrap_or(false);
    if dark { th.set_mode(2); }
    let sfx = if dark { "dark" } else { "light" };
    save(&win, w, h, &format!("gallery-{sfx}"));
    // viewer
    app.set_view(1);
    app.set_view_image(pic(11, 1200, 800));
    app.set_view_w(1200); app.set_view_h(800);
    app.set_view_name("Dalat trip 12.jpg".into());
    app.set_view_index_text("3 of 19".into());
    app.set_has_prev(true); app.set_has_next(true);
    app.set_info_open(true);
    app.set_hist_luma("M 0 100 L 40 30 L 120 60 L 200 20 L 255 100 Z".into());
    app.set_info_rows(ModelRc::new(VecModel::from(vec![InfoRow { label: "Name".into(), value: "Dalat trip 12.jpg".into() }, InfoRow { label: "Dimensions".into(), value: "4032 × 3024".into() }, InfoRow { label: "Camera".into(), value: "Apple iPhone 15".into() }])));
    let strip: Vec<Strip> = (0..9).map(|i| Strip { idx: i, thumb: pic(i as u32 * 5 + 1, 100, 100), current: i == 4, loaded: true }).collect();
    app.set_strip(ModelRc::new(VecModel::from(strip)));
    save(&win, w, h, &format!("viewer-{sfx}"));
    // settings
    app.set_view(0);
    let sw: Vec<slint::Color> = ["#CE422B", "#F74C00", "#B7410E", "#DEA584", "#0078D4", "#0063B1", "#0099BC", "#00B7C3", "#038387", "#10893E", "#6B69D6", "#8764B8", "#B146C2", "#E3008C", "#D13438", "#DA3B01", "#F7630C", "#FF8C00", "#68768A"].iter().map(|h| { let n = u32::from_str_radix(&h[1..], 16).unwrap(); slint::Color::from_rgb_u8((n >> 16) as u8, (n >> 8) as u8, n as u8) }).collect();
    app.set_swatches(ModelRc::new(VecModel::from(sw)));
    app.set_settings_open(true);
    save(&win, w, h, &format!("settings-{sfx}"));

    // editor
    app.set_settings_open(false);
    app.set_view(2);
    app.set_ed_image(pic(21, 1200, 800));
    app.set_ed_w(1200); app.set_ed_h(800);
    app.set_ed_title("Dalat trip 12.jpg".into());
    app.set_ed_dirty(true); app.set_ed_can_undo(true);
    app.set_ed_adj(ModelRc::new(VecModel::from(vec![10.0f32, -20.0, 0.0, 35.0, 0.0, 0.0, 12.0, 30.0, -25.0, 0.0, 0.0])));
    app.set_ed_cx(0.12); app.set_ed_cy(0.1); app.set_ed_cw(0.7); app.set_ed_ch(0.75);
    app.set_ed_size_text("2823 × 2268 px".into());
    let fl: Vec<FilterItem> = ["Original", "Vivid", "Warm", "Cool", "Mono", "Noir", "Sepia"].iter().enumerate().map(|(i, n)| FilterItem { id: if i == 0 { "none".into() } else { format!("f{i}").into() }, label: (*n).into(), thumb: pic(i as u32 * 9 + 2, 160, 160) }).collect();
    app.set_ed_filters(ModelRc::new(VecModel::from(fl)));
    app.set_ed_layers(ModelRc::new(VecModel::from(vec![LayerItem { id: 1, label: "Pen".into(), kind: 0 }, LayerItem { id: 2, label: "Your text".into(), kind: 2 }])));
    for (tab, name) in [(0, "crop"), (1, "adjust"), (2, "filters"), (3, "draw")] {
        app.set_ed_tab(tab);
        if tab == 3 { app.set_ed_tool(7); app.set_ed_text_selected(true); app.set_ed_text("Hello Dalat".into()); }
        if tab == 2 { app.set_ed_filter("f2".into()); }
        save(&win, w, h, &format!("editor-{name}-{sfx}"));
    }
}
