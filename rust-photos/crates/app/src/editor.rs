//! Editor logic: a worker thread owns the pixel engine, the UI thread owns the edit state and its history.
use crate::state::{post, State};
use crate::util::*;
use crate::{App, FilterItem, LayerItem};
use rust_photos_core::edit::{self, draw, Adj, BlurMode, Crop, EditState, Engine, ExportOpts, Format, Item, RenderOpts, ShapeKind, ADJ_KEYS, FILTER_IDS};
use rust_photos_core::decode;
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

pub enum Msg {
    Render { st: EditState, max: u32, opts: RenderOpts, gen: u64 },
    Auto(EditState),
    Save { st: EditState, o: ExportOpts, path: PathBuf, exif: Option<PathBuf>, overwrite: bool },
}

pub struct Editor {
    pub path: String,
    pub tx: Sender<Msg>,
    pub st: EditState,
    pub saved: EditState,
    pub hist: Vec<EditState>,
    pub hi: usize,
    pub gen: u64,
    pub draft: Option<Item>,
    pub start: Option<[f32; 2]>,
    pub next_id: u64,
    pub sel: Option<u64>,
    pub orig: (u32, u32),
    pub compare: bool,
    pub drag: Option<[f32; 2]>,
    pub coalesce: Option<String>,
    pub preview: (u32, u32),
}

const PREVIEW_MAX: u32 = 1600;

fn worker(path: String, rx: Receiver<Msg>) {
    let img = match decode::open_upright(Path::new(&path)) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            post(move |s| s.ed_failed(e));
            return;
        }
    };
    let (w, h) = img.dimensions();
    let mut eng = Engine::new(img);
    let filters: Vec<(String, SharedPixelBuffer<Rgba8Pixel>)> = FILTER_IDS.iter().map(|f| (f.to_string(), to_buf(&eng.filter_preview(f, 160)))).collect();
    post(move |s| s.ed_ready(w, h, filters));
    loop {
        let Ok(first) = rx.recv() else { break };
        let mut msgs = vec![first];
        while let Ok(m) = rx.try_recv() { msgs.push(m); }
        // only the newest render matters while the user is dragging a slider
        let last_render = msgs.iter().rposition(|m| matches!(m, Msg::Render { .. }));
        for (i, m) in msgs.into_iter().enumerate() {
            match m {
                Msg::Render { st, max, opts, gen } => {
                    if Some(i) != last_render { continue; }
                    let img = eng.preview(&st, max, &opts);
                    let (w, h) = img.dimensions();
                    let buf = to_buf(&img);
                    post(move |s| s.ed_rendered(gen, buf, w, h));
                }
                Msg::Auto(st) => {
                    let a = eng.auto_enhance(&st);
                    post(move |s| s.ed_auto_done(a));
                }
                Msg::Save { st, o, path, exif, overwrite } => {
                    let r = edit::save(&mut eng, &st, &o, &path, exif.as_deref());
                    let p = path.to_string_lossy().into_owned();
                    post(move |s| s.ed_saved(r, p, overwrite));
                }
            }
        }
    }
}

fn aspect_ratio(i: i32, orig: f32) -> f32 {
    match i { 1 => orig, 2 => 1.0, 3 => 4.0 / 3.0, 4 => 3.0 / 2.0, 5 => 16.0 / 9.0, 6 => 9.0 / 16.0, _ => 0.0 }
}

impl State {
    pub fn open_editor(&mut self) {
        let Some(path) = self.current_path() else { return };
        let (tx, rx) = channel();
        let p2 = path.clone();
        std::thread::spawn(move || worker(p2, rx));
        let st = EditState::default();
        self.ed = Some(Editor {
            path: path.clone(), tx, saved: st.clone(), hist: vec![st.clone()], st, hi: 0, gen: 0, draft: None, start: None, next_id: 1, sel: None,
            orig: (0, 0), compare: false, drag: None, coalesce: None, preview: (0, 0),
        });
        let ui = self.ui();
        let app = ui.global::<App>();
        app.set_ed_w(0);
        app.set_ed_image(Image::default());
        app.set_ed_tab(0);
        app.set_ed_tool(0);
        app.set_ed_aspect(0);
        app.set_ed_aspect_ratio(0.0);
        app.set_ed_title(base_name(&path).into());
        app.set_ed_quality(self.settings.quality as i32);
        let ext = Path::new(&path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        app.set_ed_fmt(match Format::from_ext(&ext) { Format::Png => 0, Format::Jpeg => 1, Format::Webp => 2 });
        app.set_view(2);
        self.sync_editor_ui();
    }

    pub fn ed_failed(&mut self, e: String) {
        self.ed = None;
        self.ui().global::<App>().set_view(1);
        self.error(&e);
    }

    pub fn ed_ready(&mut self, w: u32, h: u32, filters: Vec<(String, SharedPixelBuffer<Rgba8Pixel>)>) {
        let Some(ed) = self.ed.as_mut() else { return };
        ed.orig = (w, h);
        let items: Vec<FilterItem> = filters
            .into_iter()
            .map(|(id, b)| FilterItem { label: i18n_flt(self.settings.lang, &id).into(), id: id.into(), thumb: Image::from_rgba8(b) })
            .collect();
        self.ui().global::<App>().set_ed_filters(ModelRc::new(VecModel::from(items)));
        self.ed_render();
        self.sync_editor_ui();
    }

    pub fn ed_render(&mut self) {
        let tab = self.ui().global::<App>().get_ed_tab();
        let Some(ed) = self.ed.as_mut() else { return };
        ed.gen += 1;
        let opts = RenderOpts { full: tab == 0, plain: ed.compare, draft: ed.draft.clone(), hide_id: None };
        let _ = ed.tx.send(Msg::Render { st: ed.st.clone(), max: PREVIEW_MAX, opts, gen: ed.gen });
    }

    pub fn ed_rendered(&mut self, gen: u64, buf: SharedPixelBuffer<Rgba8Pixel>, w: u32, h: u32) {
        let Some(ed) = self.ed.as_mut() else { return };
        if gen != ed.gen { return; }
        ed.preview = (w, h);
        let ui = self.ui();
        let app = ui.global::<App>();
        app.set_ed_image(Image::from_rgba8(buf));
        app.set_ed_w(w as i32);
        app.set_ed_h(h as i32);
    }

    /// Pushes the edit state into the properties the editor screen shows.
    pub fn sync_editor_ui(&mut self) {
        let ui = self.ui();
        let app = ui.global::<App>();
        let lang = self.settings.lang;
        let Some(ed) = self.ed.as_ref() else { return };
        let st = &ed.st;
        app.set_ed_adj(ModelRc::new(VecModel::from(ADJ_KEYS.iter().map(|k| st.adj.get(k)).collect::<Vec<f32>>())));
        app.set_ed_angle(st.angle);
        app.set_ed_cx(st.crop.x);
        app.set_ed_cy(st.crop.y);
        app.set_ed_cw(st.crop.w);
        app.set_ed_ch(st.crop.h);
        app.set_ed_filter(st.filter.clone().into());
        app.set_ed_filter_amt(st.filter_amt);
        app.set_ed_can_undo(ed.hi > 0);
        app.set_ed_can_redo(ed.hi + 1 < ed.hist.len());
        app.set_ed_dirty(*st != ed.saved);
        let (ow, oh) = if st.rot % 2 == 1 { (ed.orig.1, ed.orig.0) } else { ed.orig };
        app.set_ed_size_text(format!("{} × {} px", (ow as f32 * st.crop.w).round() as u32, (oh as f32 * st.crop.h).round() as u32).into());
        let layers: Vec<LayerItem> = st
            .items
            .iter()
            .map(|it| {
                let (kind, label) = match it {
                    Item::Stroke { highlighter, .. } => (0, i18n_t(lang, if *highlighter { "draw.highlighter" } else { "draw.pen" })),
                    Item::Shape { shape, .. } => (1, i18n_t(lang, match shape { ShapeKind::Line => "draw.line", ShapeKind::Arrow => "draw.arrow", ShapeKind::Rect => "draw.rect", ShapeKind::Oval => "draw.oval" })),
                    Item::Text { text, .. } => (2, text.lines().next().unwrap_or("").chars().take(24).collect()),
                    Item::Blur { mode, .. } => (3, i18n_t(lang, if matches!(mode, BlurMode::Pixelate) { "draw.pixelate" } else { "draw.blurArea" })),
                    Item::Emoji { ch, .. } => (4, ch.clone()),
                };
                LayerItem { id: it.id() as i32, label: label.into(), kind }
            })
            .collect();
        app.set_ed_layers(ModelRc::new(VecModel::from(layers)));
        match ed.sel.and_then(|id| st.items.iter().find(|i| i.id() == id)) {
            Some(Item::Text { text, size, color, bold, italic, bg, .. }) => {
                app.set_ed_text_selected(true);
                app.set_ed_text(text.clone().into());
                app.set_ed_text_size(size * 100.0);
                app.set_ed_color(slint::Color::from_rgb_u8(color[0], color[1], color[2]));
                app.set_ed_bold(*bold);
                app.set_ed_italic(*italic);
                app.set_ed_text_bg(*bg);
            }
            _ => app.set_ed_text_selected(false),
        }
        app.set_ed_sel_layer(ed.sel.map(|i| i as i32).unwrap_or(-1));
    }

    fn ed_push(&mut self, coalesce: Option<String>) {
        if let Some(ed) = self.ed.as_mut() {
            if ed.hist.get(ed.hi) == Some(&ed.st) { return; }
            if coalesce.is_some() && ed.coalesce == coalesce && ed.hi + 1 == ed.hist.len() {
                let st = ed.st.clone();
                ed.hist[ed.hi] = st;
            } else {
                ed.hist.truncate(ed.hi + 1);
                ed.hist.push(ed.st.clone());
                ed.hi += 1;
                if ed.hist.len() > 100 { ed.hist.remove(0); ed.hi -= 1; }
            }
            ed.coalesce = coalesce;
        }
        self.sync_editor_ui();
    }

    fn ed_apply(&mut self, f: impl FnOnce(&mut EditState)) {
        if let Some(ed) = self.ed.as_mut() { f(&mut ed.st); }
        self.ed_render();
        self.sync_editor_ui();
    }

    pub fn ed_adj_changed(&mut self, i: usize, v: f32) {
        if let Some(k) = ADJ_KEYS.get(i) { self.ed_apply(|s| s.adj.set(k, v)); }
    }
    pub fn ed_angle_changed(&mut self, v: f32) { self.ed_apply(|s| s.angle = v); }
    pub fn ed_commit(&mut self) { self.ed_push(None); }

    pub fn ed_crop_changed(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if let Some(ed) = self.ed.as_mut() { ed.st.crop = Crop { x, y, w, h }; }
        self.sync_editor_ui();
    }

    pub fn ed_auto_done(&mut self, a: Adj) {
        self.ed_apply(|s| s.adj = Adj { sharpness: s.adj.sharpness, vignette: s.adj.vignette, hue: s.adj.hue, blur: s.adj.blur, warmth: s.adj.warmth, tint: s.adj.tint, ..a });
        self.ed_push(None);
    }

    pub fn ed_text_edited(&mut self, t: String) {
        let Some(id) = self.ed.as_ref().and_then(|e| e.sel) else { return };
        self.ed_apply(|s| { if let Some(Item::Text { text, .. }) = s.items.iter_mut().find(|i| i.id() == id) { *text = t; } });
        self.ed_push(Some(format!("text-{id}")));
    }

    /// Colour, size and style controls: they change the selected text.
    pub fn ed_style_changed(&mut self) {
        let ui = self.ui();
        let app = ui.global::<App>();
        let c = app.get_ed_color();
        let (size, bold, italic, bg) = (app.get_ed_text_size() / 100.0, app.get_ed_bold(), app.get_ed_italic(), app.get_ed_text_bg());
        let Some(id) = self.ed.as_ref().and_then(|e| e.sel) else { return };
        self.ed_apply(|s| {
            if let Some(Item::Text { size: sz, color, bold: b, italic: it, bg: g, .. }) = s.items.iter_mut().find(|i| i.id() == id) {
                *sz = size; *color = [c.red(), c.green(), c.blue(), 255]; *b = bold; *it = italic; *g = bg;
            }
        });
        self.ed_push(Some(format!("style-{id}")));
    }

    pub fn ed_canvas(&mut self, kind: i32, x: f32, y: f32) {
        let ui = self.ui();
        let app = ui.global::<App>();
        let tool = app.get_ed_tool();
        let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
        let c = app.get_ed_color();
        let color = [c.red(), c.green(), c.blue(), 255];
        let (width, fill) = (app.get_ed_width(), app.get_ed_fill());
        let text_default = self.tr("text.default");
        let (size, bold, italic, bg) = (app.get_ed_text_size() / 100.0, app.get_ed_bold(), app.get_ed_italic(), app.get_ed_text_bg());
        let Some(ed) = self.ed.as_mut() else { return };
        let mut commit = false;
        match (tool, kind) {
            (1 | 2, 0) => { ed.draft = Some(Item::Stroke { id: ed.next_id, pts: vec![[x, y]], color, width, highlighter: tool == 2 }); }
            (1 | 2, 1) => {
                if let Some(Item::Stroke { pts, .. }) = ed.draft.as_mut() {
                    let l = pts[pts.len() - 1];
                    if (l[0] - x).hypot(l[1] - y) > 0.002 { pts.push([x, y]); }
                }
            }
            (3..=6, 0) => {
                let shape = match tool { 3 => ShapeKind::Line, 4 => ShapeKind::Arrow, 5 => ShapeKind::Rect, _ => ShapeKind::Oval };
                ed.draft = Some(Item::Shape { id: ed.next_id, shape, a: [x, y], b: [x, y], color, width, fill: fill && tool >= 5 });
            }
            (3..=6, 1) => { if let Some(Item::Shape { b, .. }) = ed.draft.as_mut() { *b = [x, y]; } }
            (8 | 9, 0) => {
                ed.start = Some([x, y]);
                ed.draft = Some(Item::Blur { id: ed.next_id, x, y, w: 0.0, h: 0.0, mode: if tool == 8 { BlurMode::Pixelate } else { BlurMode::Blur } });
            }
            (8 | 9, 1) => {
                if let (Some(s), Some(Item::Blur { x: bx, y: by, w, h, .. })) = (ed.start, ed.draft.as_mut()) {
                    *bx = s[0].min(x); *by = s[1].min(y); *w = (s[0] - x).abs(); *h = (s[1] - y).abs();
                }
            }
            (7, 0) => {
                let (pw, ph) = (ed.preview.0.max(1) as f32, ed.preview.1.max(1) as f32);
                let hit = ed.st.items.iter().rev().find(|it| match it {
                    Item::Text { text, x: tx, y: ty, size, bold, italic, .. } => {
                        let (bw, bh, _) = draw::text_box(text, size * pw, *bold, *italic);
                        (x - tx).abs() <= bw / (2.0 * pw) && (y - ty).abs() <= bh / (2.0 * ph)
                    }
                    _ => false,
                });
                if let Some(Item::Text { id, x: tx, y: ty, .. }) = hit {
                    ed.sel = Some(*id);
                    ed.drag = Some([x - tx, y - ty]);
                } else {
                    let id = ed.next_id;
                    ed.next_id += 1;
                    ed.st.items.push(Item::Text { id, text: text_default, x, y, size, rot: 0.0, color, bold, italic, bg });
                    ed.sel = Some(id);
                    ed.drag = Some([0.0, 0.0]);
                }
            }
            (7, 1) => {
                if let (Some(off), Some(id)) = (ed.drag, ed.sel) {
                    if let Some(Item::Text { x: tx, y: ty, .. }) = ed.st.items.iter_mut().find(|i| i.id() == id) { *tx = x - off[0]; *ty = y - off[1]; }
                }
            }
            (_, 2) => {
                commit = true;
                if let Some(d) = ed.draft.take() {
                    let ok = match &d {
                        Item::Shape { a, b, .. } => (a[0] - b[0]).hypot(a[1] - b[1]) > 0.005,
                        Item::Blur { w, h, .. } => *w > 0.01 && *h > 0.01,
                        _ => true,
                    };
                    if ok { ed.next_id += 1; ed.st.items.push(d); }
                }
                ed.drag = None;
                ed.start = None;
            }
            _ => {}
        }
        self.ed_render();
        if commit { self.ed_push(None); } else { self.sync_editor_ui(); }
    }

    pub fn ed_act(&mut self, name: &str, i: i32) {
        let ui = self.ui();
        let app = ui.global::<App>();
        match name {
            "ed-back" => {
                if self.ed.as_ref().map(|e| e.st != e.saved).unwrap_or(false) {
                    let (t, d, y) = (self.tr("ed.discardTitle"), self.tr("ed.discardDesc"), self.tr("ed.discard"));
                    self.confirm(t, d, y, true, |s| s.ed_close());
                } else { self.ed_close(); }
            }
            "ed-tab" => { app.set_ed_tab(i); self.ed_render(); }
            "ed-tool" => { let t = if app.get_ed_tool() == i { 0 } else { i }; app.set_ed_tool(t); if let Some(e) = self.ed.as_mut() { e.sel = None; } self.sync_editor_ui(); }
            "ed-undo" => { if let Some(e) = self.ed.as_mut() { if e.hi > 0 { e.hi -= 1; e.st = e.hist[e.hi].clone(); e.coalesce = None; } } self.ed_render(); self.sync_editor_ui(); }
            "ed-redo" => { if let Some(e) = self.ed.as_mut() { if e.hi + 1 < e.hist.len() { e.hi += 1; e.st = e.hist[e.hi].clone(); e.coalesce = None; } } self.ed_render(); self.sync_editor_ui(); }
            "ed-reset" => { self.ed_apply(|s| *s = EditState::default()); app.set_ed_aspect(0); app.set_ed_aspect_ratio(0.0); self.ed_push(None); }
            "ed-rot" => { self.ed_apply(|s| { s.rot = (s.rot + i as u8) % 4; s.crop = Crop::default(); }); app.set_ed_aspect(0); app.set_ed_aspect_ratio(0.0); self.ed_push(None); }
            "ed-flip" => { self.ed_apply(|s| { if i == 0 { s.flip_h = !s.flip_h; } else { s.flip_v = !s.flip_v; } s.crop = Crop::default(); }); self.ed_push(None); }
            "ed-aspect" => {
                app.set_ed_aspect(i);
                let (iw, ih) = (app.get_ed_w().max(1) as f32, app.get_ed_h().max(1) as f32);
                let ratio = aspect_ratio(i, iw / ih);
                app.set_ed_aspect_ratio(ratio);
                if ratio > 0.0 {
                    let r = ratio * ih / iw;
                    let (w, h) = if r >= 1.0 { (1.0, 1.0 / r) } else { (r, 1.0) };
                    self.ed_apply(|s| s.crop = Crop { x: (1.0 - w) / 2.0, y: (1.0 - h) / 2.0, w, h });
                    self.ed_push(None);
                }
            }
            "ed-auto" => { if let Some(e) = self.ed.as_ref() { let _ = e.tx.send(Msg::Auto(e.st.clone())); } }
            "ed-adj-reset" => { self.ed_apply(|s| s.adj = Adj::default()); self.ed_push(None); }
            "ed-filter" => { if let Some(id) = FILTER_IDS.get(i as usize) { let id = id.to_string(); self.ed_apply(|s| s.filter = id); self.ed_push(None); } }
            "ed-filter-amt" => self.ed_apply(|s| s.filter_amt = i as f32),
            "ed-compare" => { if let Some(e) = self.ed.as_mut() { e.compare = i == 1; } self.ed_render(); }
            "ed-layer-select" => { if let Some(e) = self.ed.as_mut() { e.sel = Some(i as u64); } self.sync_editor_ui(); }
            "ed-layer-delete" => {
                let id = i as u64;
                if self.ed.as_ref().and_then(|e| e.sel) == Some(id) { if let Some(e) = self.ed.as_mut() { e.sel = None; } }
                self.ed_apply(|s| s.items.retain(|it| it.id() != id));
                self.ed_push(None);
            }
            "ed-save" => self.ed_save(true),
            "ed-save-copy" => self.ed_save(false),
            _ => {}
        }
    }

    pub fn ed_close(&mut self) {
        self.ed = None;
        self.ui().global::<App>().set_view(1);
    }

    fn ed_opts(&self, fmt: Format) -> ExportOpts {
        let q = self.ui().global::<App>().get_ed_quality().clamp(50, 100) as u8;
        ExportOpts { format: fmt, quality: q, size: None }
    }

    fn ed_save(&mut self, overwrite: bool) {
        let Some(ed) = self.ed.as_ref() else { return };
        if ed.st == ed.saved { return; }
        let path = ed.path.clone();
        let ext = Path::new(&path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let can_overwrite = matches!(ext.as_str(), "jpg" | "jpeg" | "jfif" | "png" | "webp");
        if overwrite && can_overwrite {
            let (t, d, y) = (self.tr("exp.overwriteTitle"), self.trf("exp.overwriteDesc", &[("name", &base_name(&path))]), self.tr("exp.overwriteConfirm"));
            self.confirm(t, d, y, true, move |s| {
                let fmt = Format::from_ext(&ext);
                s.ed_send_save(PathBuf::from(&path), fmt, true);
            });
            return;
        }
        // save a copy: ask where
        let fmt = match self.ui().global::<App>().get_ed_fmt() { 0 => Format::Png, 2 => Format::Webp, _ => Format::Jpeg };
        let stem = Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = Path::new(&path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
        slint::spawn_local(async move {
            let h = rfd::AsyncFileDialog::new()
                .set_directory(&dir)
                .set_file_name(format!("{stem}-edited.{}", fmt.ext()))
                .add_filter("PNG", &["png"]).add_filter("JPEG", &["jpg", "jpeg"]).add_filter("WebP", &["webp"])
                .save_file().await;
            if let Some(h) = h {
                let p = h.path().to_path_buf();
                let f = p.extension().map(|e| Format::from_ext(&e.to_string_lossy())).unwrap_or(fmt);
                let p = if p.extension().is_none() { p.with_extension(f.ext()) } else { p };
                crate::state::run(move |s| s.ed_send_save(p, f, false));
            }
        }).ok();
    }

    fn ed_send_save(&mut self, path: PathBuf, fmt: Format, overwrite: bool) {
        let o = self.ed_opts(fmt);
        let keep = self.settings.keep_exif;
        let Some(ed) = self.ed.as_ref() else { return };
        let exif = if keep && fmt == Format::Jpeg { Some(PathBuf::from(&ed.path)) } else { None };
        let _ = ed.tx.send(Msg::Save { st: ed.st.clone(), o, path, exif, overwrite });
        self.ui().global::<App>().set_ed_busy(true);
    }

    pub fn ed_saved(&mut self, r: Result<(), String>, path: String, overwrite: bool) {
        self.ui().global::<App>().set_ed_busy(false);
        match r {
            Err(e) => self.error(&e),
            Ok(()) => {
                if let Some(ed) = self.ed.as_mut() { ed.saved = ed.st.clone(); }
                if overwrite {
                    // the engine still holds the old pixels, so leave the editor and show the new file
                    self.ed = None;
                    self.ui().global::<App>().set_view(1);
                    self.file_changed(&path);
                    let m = self.tr("toast.saved");
                    self.toast(1, m, None);
                } else {
                    self.sync_editor_ui();
                    self.refresh();
                    let m = self.trf("toast.savedAs", &[("name", &base_name(&path))]);
                    self.toast(1, m, None);
                }
            }
        }
    }
}

fn i18n_t(lang: rust_photos_core::i18n::Lang, k: &str) -> String { rust_photos_core::i18n::tr(lang, k) }
fn i18n_flt(lang: rust_photos_core::i18n::Lang, id: &str) -> String { rust_photos_core::i18n::tr(lang, &format!("flt.{id}")) }
