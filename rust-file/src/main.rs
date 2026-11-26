#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

mod app;
mod dialogs;
mod fsops;
mod i18n;
mod icons;
mod model;
mod settings;
mod ui;

use eframe::egui;

impl eframe::App for app::App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_style(ctx);
        self.pump(ctx);
        // refresh the drive list now and then (USB sticks come and go)
        if self.drives_at.elapsed().as_secs() > 20 {
            self.drives = fsops::get_drives();
            self.drives_at = std::time::Instant::now();
        }
        self.keyboard(ctx);
        self.top_bar(ctx);
        self.status_bar(ctx);
        self.sidebar(ctx);
        self.details_pane(ctx);
        self.central(ctx);
        self.show_dialogs(ctx);
        self.show_palette(ctx);
        self.overlays(ctx);
        self.run_acts(ctx);
        self.tick(ctx);
        // remember the window size
        let (sz, max) = ctx.input(|i| (i.viewport().inner_rect.map(|r| r.size()), i.viewport().maximized));
        if let Some(sz) = sz {
            if max != Some(true) && ((sz.x - self.s.win_size[0]).abs() > 2.0 || (sz.y - self.s.win_size[1]).abs() > 2.0) {
                self.s.win_size = [sz.x, sz.y];
                self.dirty = true;
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_now();
    }
}

fn load_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = (img.width(), img.height());
    Some(egui::IconData { rgba: img.into_raw(), width: w, height: h })
}

fn main() -> eframe::Result {
    let start = std::env::args().nth(1).map(std::path::PathBuf::from);
    let s = settings::Settings::load();
    let mut vb = egui::ViewportBuilder::default()
        .with_title("Rust File")
        .with_inner_size(s.win_size)
        .with_min_inner_size([720.0, 420.0])
        .with_drag_and_drop(true);
    if let Some(i) = load_icon() {
        vb = vb.with_icon(std::sync::Arc::new(i));
    }
    let opts = eframe::NativeOptions { viewport: vb, renderer: eframe::Renderer::Glow, vsync: true, ..Default::default() };
    eframe::run_native(
        "Rust File",
        opts,
        Box::new(move |cc| {
            icons::setup_fonts(&cc.egui_ctx);
            Ok(Box::new(app::App::new(cc.egui_ctx.clone(), start)))
        }),
    )
}
