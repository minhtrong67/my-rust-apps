//! The visible UI: top bar, sidebar, file list / icon grid, details pane, overlays.

use crate::{
    app::*,
    dialogs::*,
    fsops::*,
    i18n::t,
    icons::*,
    model::*,
    settings::*,
};
use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Key, Layout, Margin, Modifiers, PointerButton, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui,
};
use egui_extras::{Column, TableBuilder};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub struct PaneCx<'a> {
    pub s: &'a Settings,
    pub acts: &'a mut Vec<Act>,
    pub thumbs: &'a mut Thumbs,
    pub clip: &'a Option<Clip>,
    pub bus: Bus,
    pub idx: usize,
    pub dual: bool,
    pub dark: bool,
    pub drives: &'a [Drive],
    pub places: &'a [(&'static str, PathBuf)],
    pub tools: (bool, bool),
}

fn accent(s: &Settings) -> Color32 {
    Color32::from_rgb(s.accent[0], s.accent[1], s.accent[2])
}

fn name_shown(e: &Entry, s: &Settings) -> String {
    if s.show_ext || e.is_dir {
        e.name.clone()
    } else {
        split_name(&e.name, false).0
    }
}

fn paint_entry_icon(p: &egui::Painter, r: Rect, e: &Entry, s: &Settings, dark: bool) {
    if e.is_dir {
        let c = s.folder_colors.get(e.path.to_string_lossy().as_ref()).map(|c| Color32::from_rgb(c[0], c[1], c[2])).unwrap_or(kind_color(Kind::Folder));
        paint_folder(p, r, c);
    } else {
        paint_file(p, r, &e.ext, e.kind(), dark);
    }
}

fn tag_color(i: u8) -> Color32 {
    let c = TAG_COLORS[(i as usize).min(TAG_COLORS.len() - 1)];
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Plain / ctrl / shift click selection.
fn click_select(tab: &mut Tab, vi: usize, m: Modifiers) {
    let paths: Vec<PathBuf> = tab.view.iter().map(|&i| tab.list()[i].path.clone()).collect();
    if vi >= paths.len() {
        return;
    }
    if m.shift {
        let a = tab.anchor.unwrap_or(vi);
        let (lo, hi) = (a.min(vi), a.max(vi));
        if !m.command {
            tab.sel.clear();
        }
        for k in lo..=hi {
            tab.sel.insert(paths[k].clone());
        }
    } else if m.command {
        if !tab.sel.remove(&paths[vi]) {
            tab.sel.insert(paths[vi].clone());
        }
        tab.anchor = Some(vi);
    } else {
        tab.sel.clear();
        tab.sel.insert(paths[vi].clone());
        tab.anchor = Some(vi);
    }
}

/// Shared mouse behaviour of a row / tile.
fn interact(tab: &mut Tab, cx: &mut PaneCx, resp: &egui::Response, e: &Entry, vi: usize) {
    let m = resp.ctx.input(|i| i.modifiers);
    if resp.clicked() || resp.secondary_clicked() && !tab.sel.contains(&e.path) || resp.drag_started() && !tab.sel.contains(&e.path) {
        click_select(tab, vi, if resp.secondary_clicked() || resp.drag_started() { Modifiers::NONE } else { m });
    }
    if resp.is_pointer_button_down_on() || resp.clicked() {
        cx.acts.push(Act::Focus(cx.idx));
    }
    if resp.double_clicked() {
        cx.acts.push(Act::Open(cx.idx, e.path.clone()));
    }
    if resp.middle_clicked() && e.is_dir {
        cx.acts.push(Act::NewTab(Loc::Dir(e.path.clone())));
    }
    if resp.drag_started() {
        let paths = tab.selected_paths();
        resp.dnd_set_drag_payload(paths);
    }
    if e.is_dir {
        if let Some(p) = resp.dnd_release_payload::<Vec<PathBuf>>() {
            let copy = resp.ctx.input(|i| i.modifiers.ctrl);
            cx.acts.push(Act::Drop { paths: (*p).clone(), dest: e.path.clone(), copy });
        }
    }
}

fn mi(ui: &mut Ui, label: &str) -> bool {
    let r = ui.button(label).clicked();
    if r {
        ui.close_menu();
    }
    r
}

/// Right-click menu for the selected items.
fn item_menu(ui: &mut Ui, tab: &Tab, cx: &mut PaneCx, e: &Entry) {
    let paths = tab.selected_paths();
    let paths = if paths.is_empty() { vec![e.path.clone()] } else { paths };
    let multi = paths.len() > 1;
    let p = cx.idx;
    ui.set_min_width(210.0);
    if mi(ui, t("Open")) {
        cx.acts.push(Act::Open(p, e.path.clone()));
    }
    if e.is_dir && !multi {
        if mi(ui, t("Open in new tab")) {
            cx.acts.push(Act::NewTab(Loc::Dir(e.path.clone())));
        }
        if mi(ui, t("Open in new window")) {
            cx.acts.push(Act::NewWindow(Some(e.path.clone())));
        }
        if cx.dual && mi(ui, t("Open in other pane")) {
            cx.acts.push(Act::OpenOtherPane(e.path.clone()));
        }
    }
    if !multi {
        ui.menu_button(t("Open with"), |ui| {
            for (l, k) in [(t("Terminal here"), "terminal"), (t("PowerShell here"), "powershell"), (t("VS Code"), "code"), (t("Notepad"), "notepad"), (t("Open with WinRAR"), "winrar")] {
                if mi(ui, l) {
                    cx.acts.push(Act::OpenWith(k, e.path.clone()));
                }
            }
        });
        for (i, c) in cx.s.custom_cmds.iter().enumerate() {
            if !c.name.is_empty() && mi(ui, &c.name) {
                cx.acts.push(Act::Custom(i, e.path.clone()));
            }
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        for (i, tip, cut) in [(Ic::Cut, t("Cut"), true), (Ic::Copy, t("Copy"), false)] {
            if ui.add(egui::Button::new(ic(i))).on_hover_text(tip).clicked() {
                cx.acts.push(Act::Copy(paths.clone(), cut));
                ui.close_menu();
            }
        }
        if cx.clip.is_some() && e.is_dir && !multi && ui.add(egui::Button::new(ic(Ic::Paste))).on_hover_text(t("Paste")).clicked() {
            cx.acts.push(Act::Navigate(p, Loc::Dir(e.path.clone())));
            cx.acts.push(Act::Paste(p));
            ui.close_menu();
        }
        if ui.add(egui::Button::new(ic(Ic::Rename))).on_hover_text(t("Rename")).clicked() && !multi {
            cx.acts.push(Act::Rename(p, e.path.clone()));
            ui.close_menu();
        }
        if ui.add(egui::Button::new(ic(Ic::Delete))).on_hover_text(t("Delete")).clicked() {
            cx.acts.push(Act::Delete(paths.clone(), false));
            ui.close_menu();
        }
    });
    if mi(ui, t("Delete permanently")) {
        cx.acts.push(Act::Delete(paths.clone(), true));
    }
    ui.separator();
    if multi && mi(ui, t("Batch rename")) {
        cx.acts.push(Act::Batch(paths.clone()));
    }
    if mi(ui, t("Compress to ZIP")) {
        cx.acts.push(Act::Compress(paths.clone()));
    }
    if !multi && !e.is_dir && e.kind() == Kind::Archive {
        if mi(ui, t("Extract here")) {
            cx.acts.push(Act::Extract(e.path.clone(), false));
        }
        if mi(ui, t("Extract to folder")) {
            cx.acts.push(Act::Extract(e.path.clone(), true));
        }
    }
    if !multi && is_image_ext(&e.ext) && mi(ui, t("Set as wallpaper")) {
        cx.acts.push(Act::Wallpaper(e.path.clone()));
    }
    ui.separator();
    let all_starred = paths.iter().all(|p| cx.s.starred(p));
    if mi(ui, if all_starred { t("Remove star") } else { t("Star") }) {
        cx.acts.push(Act::Star(paths.clone()));
    }
    ui.menu_button(t("Tag"), |ui| {
        if mi(ui, t("No tag")) {
            cx.acts.push(Act::Tag(paths.clone(), None));
        }
        for i in 0..TAG_COLORS.len() {
            let name = cx.s.tag_names.get(i).cloned().unwrap_or_default();
            if ui.button(RichText::new(format!("●  {}", t_dyn(&name))).color(tag_color(i as u8))).clicked() {
                cx.acts.push(Act::Tag(paths.clone(), Some(i as u8)));
                ui.close_menu();
            }
        }
    });
    if e.is_dir && !multi {
        let pinned = cx.s.pinned.iter().any(|x| Path::new(x) == e.path);
        if mi(ui, if pinned { t("Unpin") } else { t("Pin to quick access") }) {
            cx.acts.push(Act::Pin(e.path.clone(), !pinned));
        }
        ui.menu_button(t("Folder color"), |ui| {
            if mi(ui, t("Default color")) {
                cx.acts.push(Act::FolderColor(e.path.clone(), None));
            }
            ui.horizontal_wrapped(|ui| {
                for (_, c) in ACCENTS.iter() {
                    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                    ui.painter().rect_filled(r, CornerRadius::same(5), Color32::from_rgb(c[0], c[1], c[2]));
                    if resp.clicked() {
                        cx.acts.push(Act::FolderColor(e.path.clone(), Some(*c)));
                        ui.close_menu();
                    }
                }
            });
        });
    }
    ui.separator();
    if mi(ui, t("Show in file explorer")) {
        cx.acts.push(Act::Reveal(e.path.clone()));
    }
    if mi(ui, t("Copy path")) {
        cx.acts.push(Act::CopyPath(paths.clone()));
    }
    if mi(ui, t("Properties")) {
        cx.acts.push(Act::Props(e.path.clone()));
    }
}

fn t_dyn(s: &str) -> String {
    match s {
        "Red" => t("Red").into(),
        _ => s.to_string(),
    }
}

fn bg_menu(ui: &mut Ui, tab: &Tab, cx: &mut PaneCx) {
    let p = cx.idx;
    ui.set_min_width(190.0);
    if mi(ui, t("New folder")) {
        cx.acts.push(Act::NewItem(p, true));
    }
    if mi(ui, t("New file")) {
        cx.acts.push(Act::NewItem(p, false));
    }
    ui.separator();
    if ui.add_enabled(cx.clip.is_some(), egui::Button::new(t("Paste"))).clicked() {
        cx.acts.push(Act::Paste(p));
        ui.close_menu();
    }
    if mi(ui, t("Select all")) {
        cx.acts.push(Act::SelectAll(p));
    }
    if mi(ui, t("Refresh")) {
        cx.acts.push(Act::Refresh(p));
    }
    if let Some(d) = tab.cur_dir() {
        ui.separator();
        if mi(ui, t("Terminal here")) {
            cx.acts.push(Act::OpenWith("terminal", d.to_path_buf()));
        }
        if mi(ui, t("Properties")) {
            cx.acts.push(Act::Props(d.to_path_buf()));
        }
    }
}

/* ------------------------------------------------------------------ views */

fn sort_header(ui: &mut Ui, tab: &Tab, cx: &mut PaneCx, label: &str, key: SortKey) {
    let arrow = if tab.key == key {
        if tab.asc {
            " ⬆"
        } else {
            " ⬇"
        }
    } else {
        ""
    };
    let r = ui.add(egui::Label::new(RichText::new(format!("{label}{arrow}")).strong().small()).sense(Sense::click()));
    if r.clicked() {
        cx.acts.push(Act::Sort(cx.idx, key));
    }
}

fn kind_text(e: &Entry) -> String {
    if e.is_dir {
        return t("Folder").into();
    }
    match e.kind() {
        Kind::Other if !e.ext.is_empty() => format!("{} {}", e.ext.to_uppercase(), t("File").to_lowercase()),
        k => t(kind_label(k)).into(),
    }
}

fn rename_box(ui: &mut Ui, tab: &mut Tab, cx: &mut PaneCx, width: f32) {
    let Some(r) = tab.rename.as_mut() else { return };
    let id = egui::Id::new(("rename", tab.id));
    let out = egui::TextEdit::singleline(&mut r.text).id(id).desired_width(width).show(ui);
    if r.first {
        r.first = false;
        out.response.request_focus();
        let stem_len = split_name(&r.text, r.path.is_dir()).0.chars().count();
        let mut st = out.state.clone();
        st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(stem_len))));
        st.store(ui.ctx(), id);
    } else if out.response.lost_focus() {
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            cx.acts.push(Act::CancelRename(cx.idx));
        } else {
            cx.acts.push(Act::CommitRename(cx.idx));
        }
    }
}

fn details_view(ui: &mut Ui, tab: &mut Tab, cx: &mut PaneCx) {
    let row_h = if cx.s.compact { 22.0 } else { 28.0 };
    let scroll_idx = tab.scroll_to.take().and_then(|p| tab.view.iter().position(|&i| tab.list()[i].path == p));
    let avail = ui.available_width();
    let show_type = avail > 560.0;
    let name_w = (avail - 150.0 - 96.0 - if show_type { 110.0 } else { 0.0 } - 24.0).clamp(140.0, 420.0);
    let mut tb = TableBuilder::new(ui)
        .id_salt(("tbl", cx.idx))
        .striped(false)
        .resizable(true)
        .sense(Sense::click_and_drag())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::initial(name_w).at_least(120.0).clip(true))
        .column(Column::initial(140.0).at_least(80.0).clip(true));
    if show_type {
        tb = tb.column(Column::initial(110.0).at_least(60.0).clip(true));
    }
    tb = tb.column(Column::remainder().at_least(64.0).clip(true));
    if let Some(i) = scroll_idx {
        tb = tb.scroll_to_row(i, Some(Align::Center));
    }
    let n = tab.view.len();
    let show_search_path = tab.search.is_some();
    tb.header(24.0, |mut h| {
        h.col(|ui| sort_header(ui, tab, cx, t("Name"), SortKey::Name));
        h.col(|ui| sort_header(ui, tab, cx, t("Date modified"), SortKey::Modified));
        if show_type {
            h.col(|ui| sort_header(ui, tab, cx, t("Type"), SortKey::Kind));
        }
        h.col(|ui| sort_header(ui, tab, cx, t("Size"), SortKey::Size));
    })
    .body(|body| {
        body.rows(row_h, n, |mut row| {
            let vi = row.index();
            let e = tab.list()[tab.view[vi]].clone();
            let selected = tab.sel.contains(&e.path);
            row.set_selected(selected);
            let renaming = tab.rename.as_ref().map_or(false, |r| r.path == e.path);
            let cut = cx.clip.as_ref().map_or(false, |c| c.cut && c.paths.contains(&e.path));
            row.col(|ui| {
                let isz = row_h - 6.0;
                let (r, _) = ui.allocate_exact_size(vec2(isz, isz), Sense::hover());
                paint_entry_icon(ui.painter(), r, &e, cx.s, cx.dark);
                ui.add_space(4.0);
                if renaming {
                    rename_box(ui, tab, cx, ui.available_width() - 8.0);
                } else {
                    let mut txt = RichText::new(name_shown(&e, cx.s));
                    if e.hidden || cut {
                        txt = txt.weak();
                    }
                    ui.add(egui::Label::new(txt).truncate());
                    if let Some(tg) = cx.s.tag_of(&e.path) {
                        let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, tag_color(tg));
                    }
                    if cx.s.starred(&e.path) {
                        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        paint_star(ui.painter(), r.center(), 6.0, Color32::from_rgb(255, 185, 0));
                    }
                    if show_search_path {
                        if let Some(par) = e.path.parent() {
                            ui.add(egui::Label::new(RichText::new(par.display().to_string()).weak().small()).truncate());
                        }
                    }
                }
            });
            row.col(|ui| {
                ui.add(egui::Label::new(RichText::new(fmt_date(e.modified)).weak()).truncate());
            });
            if show_type {
                row.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(kind_text(&e)).weak()).truncate());
                });
            }
            row.col(|ui| {
                let s = if e.is_dir { tab.dir_sizes.get(&e.path).map(|v| fmt_size(*v)).unwrap_or_default() } else { fmt_size(e.size) };
                ui.add(egui::Label::new(RichText::new(s).weak()).truncate());
            });
            let resp = row.response();
            interact(tab, cx, &resp, &e, vi);
            resp.context_menu(|ui| item_menu(ui, tab, cx, &e));
        });
    });
}

fn grid_view(ui: &mut Ui, tab: &mut Tab, cx: &mut PaneCx) {
    let (isz, cw, ch) = match tab.mode {
        ViewMode::Small => (32.0, 100.0, 76.0),
        ViewMode::Medium => (64.0, 116.0, 118.0),
        _ => (104.0, 150.0, 164.0),
    };
    let avail = ui.available_width() - 14.0;
    let cols = ((avail / cw).floor() as usize).max(1);
    tab.cols = cols;
    let n = tab.view.len();
    let rows = (n + cols - 1) / cols;
    let target = tab.scroll_to.take().and_then(|p| tab.view.iter().position(|&i| tab.list()[i].path == p));
    let mut sa = egui::ScrollArea::vertical().id_salt(("grid", cx.idx)).auto_shrink([false, false]);
    if let Some(i) = target {
        sa = sa.vertical_scroll_offset(((i / cols) as f32 * ch - 40.0).max(0.0));
    }
    sa.show_rows(ui, ch, rows, |ui, range| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        for r in range {
            ui.horizontal(|ui| {
                for c in 0..cols {
                    let vi = r * cols + c;
                    if vi >= n {
                        break;
                    }
                    let e = tab.list()[tab.view[vi]].clone();
                    let (rect, resp) = ui.allocate_exact_size(vec2(cw, ch), Sense::click_and_drag());
                    let selected = tab.sel.contains(&e.path);
                    let p = ui.painter();
                    let inner = rect.shrink(3.0);
                    if selected {
                        p.rect_filled(inner, CornerRadius::same(6), accent(cx.s).gamma_multiply(0.28));
                        p.rect_stroke(inner, CornerRadius::same(6), Stroke::new(1.0, accent(cx.s)), StrokeKind::Inside);
                    } else if resp.hovered() {
                        p.rect_filled(inner, CornerRadius::same(6), ui.visuals().widgets.hovered.weak_bg_fill);
                    }
                    let icon_rect = Rect::from_center_size(pos2(rect.center().x, rect.min.y + 8.0 + isz / 2.0), vec2(isz, isz));
                    let mut drew = false;
                    if is_image_ext(&e.ext) && isz >= 60.0 {
                        if let Some(tex) = cx.thumbs.tex.get(&e.path) {
                            let ts = tex.size_vec2();
                            let sc = (isz / ts.x).min(isz / ts.y);
                            let r = Rect::from_center_size(icon_rect.center(), ts * sc);
                            p.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            drew = true;
                        } else {
                            cx.thumbs.request(&e.path, 256, &cx.bus);
                        }
                    }
                    if !drew {
                        paint_entry_icon(p, icon_rect, &e, cx.s, cx.dark);
                    }
                    let renaming = tab.rename.as_ref().map_or(false, |r| r.path == e.path);
                    let name_top = rect.min.y + 10.0 + isz;
                    if renaming {
                        let nr = Rect::from_min_size(pos2(rect.min.x + 4.0, name_top), vec2(cw - 8.0, 22.0));
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(nr));
                        rename_box(&mut child, tab, cx, cw - 12.0);
                    } else {
                        let mut job = egui::text::LayoutJob::simple(name_shown(&e, cx.s), FontId::proportional(12.5), ui.visuals().text_color(), cw - 14.0);
                        job.wrap.max_rows = if isz > 40.0 { 2 } else { 2 };
                        job.wrap.break_anywhere = true;
                        job.halign = Align::Center;
                        let g = ui.painter().layout_job(job);
                        ui.painter().galley(pos2(rect.center().x, name_top), g, ui.visuals().text_color());
                    }
                    if let Some(tg) = cx.s.tag_of(&e.path) {
                        ui.painter().circle_filled(pos2(rect.min.x + 12.0, rect.min.y + 12.0), 4.5, tag_color(tg));
                    }
                    if cx.s.starred(&e.path) {
                        paint_star(ui.painter(), pos2(rect.max.x - 14.0, rect.min.y + 13.0), 7.0, Color32::from_rgb(255, 185, 0));
                    }
                    interact(tab, cx, &resp, &e, vi);
                    resp.context_menu(|ui| item_menu(ui, tab, cx, &e));
                }
            });
        }
    });
}

fn card(ui: &mut Ui, size: egui::Vec2, s: &Settings) -> (Rect, egui::Response) {
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() { ui.visuals().widgets.hovered.weak_bg_fill } else { ui.visuals().faint_bg_color };
    ui.painter().rect_filled(r, CornerRadius::same(8), fill);
    ui.painter().rect_stroke(r, CornerRadius::same(8), Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), StrokeKind::Inside);
    let _ = s;
    (r, resp)
}

fn this_pc(ui: &mut Ui, cx: &mut PaneCx) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(6.0);
        ui.label(RichText::new(t("Quick access")).strong());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for (n, d) in cx.places.iter() {
                let (r, resp) = card(ui, vec2(190.0, 52.0), cx.s);
                paint_folder(ui.painter(), Rect::from_min_size(r.min + vec2(10.0, 9.0), vec2(34.0, 34.0)), kind_color(Kind::Folder));
                ui.painter().with_clip_rect(r.shrink(2.0)).text(r.min + vec2(54.0, 26.0), Align2::LEFT_CENTER, t(n), FontId::proportional(14.0), ui.visuals().text_color());
                if resp.clicked() {
                    cx.acts.push(Act::Navigate(cx.idx, Loc::Dir(d.clone())));
                }
            }
        });
        ui.add_space(14.0);
        ui.label(RichText::new(t("Drives")).strong());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for d in cx.drives.iter() {
                let (r, resp) = card(ui, vec2(270.0, 78.0), cx.s);
                paint_drive(ui.painter(), Rect::from_min_size(r.min + vec2(8.0, 8.0), vec2(48.0, 48.0)), Color32::from_rgb(120, 130, 150));
                let title = if d.name.is_empty() { d.path.display().to_string() } else { format!("{} ({})", d.name, d.path.display()) };
                ui.painter().with_clip_rect(r.shrink(2.0)).text(r.min + vec2(64.0, 16.0), Align2::LEFT_CENTER, title, FontId::proportional(13.5), ui.visuals().text_color());
                let used = d.total.saturating_sub(d.free);
                let frac = used as f32 / d.total.max(1) as f32;
                let bar = Rect::from_min_size(r.min + vec2(64.0, 32.0), vec2(194.0, 8.0));
                ui.painter().rect_filled(bar, CornerRadius::same(4), ui.visuals().widgets.inactive.bg_fill);
                let col = if frac > 0.9 { Color32::from_rgb(196, 43, 28) } else { accent(cx.s) };
                ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac, 8.0)), CornerRadius::same(4), col);
                ui.painter().text(
                    r.min + vec2(64.0, 54.0),
                    Align2::LEFT_CENTER,
                    format!("{} {} {}", fmt_size(d.free), t("free of"), fmt_size(d.total)),
                    FontId::proportional(11.5),
                    ui.visuals().weak_text_color(),
                );
                if resp.clicked() {
                    cx.acts.push(Act::Navigate(cx.idx, Loc::Dir(d.path.clone())));
                }
            }
        });
        if !cx.s.recent.is_empty() {
            ui.add_space(14.0);
            ui.label(RichText::new(t("Frequent folders")).strong());
            ui.add_space(4.0);
            for r in cx.s.recent.iter().take(8) {
                if ui.link(r).clicked() {
                    cx.acts.push(Act::Navigate(cx.idx, Loc::Dir(PathBuf::from(r))));
                }
            }
        }
    });
}

pub fn pane_ui(ui: &mut Ui, tab: &mut Tab, cx: &mut PaneCx) {
    let id = cx.idx;
    ui.push_id(("pane", id), |ui| pane_inner(ui, tab, cx));
}

fn pane_inner(ui: &mut Ui, tab: &mut Tab, cx: &mut PaneCx) {
    let rect = ui.available_rect_before_wrap();
    if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(rect) {
        cx.acts.push(Act::Focus(cx.idx));
    }
    if cx.dual {
        let focused = cx.focused_marker();
        let h = Rect::from_min_size(rect.min, vec2(rect.width(), 3.0));
        ui.painter().rect_filled(h, CornerRadius::ZERO, if focused { accent(cx.s) } else { Color32::TRANSPARENT });
        ui.add_space(4.0);
    }
    match tab.loc.clone() {
        Loc::ThisPc => this_pc(ui, cx),
        Loc::Dir(dir) => {
            // empty-space interactions (registered first so rows win)
            let bg = ui.interact(rect, ui.id().with(("bg", cx.idx)), Sense::click());
            if bg.clicked() {
                tab.sel.clear();
                tab.rename = None;
                cx.acts.push(Act::Focus(cx.idx));
            }
            if let Some(p) = bg.dnd_release_payload::<Vec<PathBuf>>() {
                let copy = ui.input(|i| i.modifiers.ctrl);
                cx.acts.push(Act::Drop { paths: (*p).clone(), dest: dir.clone(), copy });
            }
            bg.context_menu(|ui| bg_menu(ui, tab, cx));
            if let Some(err) = &tab.error {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(t("Cannot open this folder")).heading());
                    ui.label(RichText::new(err).weak());
                });
            } else if tab.loading && tab.entries.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    ui.label(t("Loading..."));
                });
            } else if tab.view.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                    paint_folder(ui.painter(), r, kind_color(Kind::Folder).gamma_multiply(0.5));
                    let running = tab.search.as_ref().map_or(false, |s| s.running);
                    ui.label(RichText::new(if running { t("Searching...") } else if tab.search.is_some() { t("No results") } else { t("This folder is empty") }).weak());
                });
            } else {
                match tab.mode {
                    ViewMode::Details => details_view(ui, tab, cx),
                    _ => grid_view(ui, tab, cx),
                }
            }
        }
    }
}

impl<'a> PaneCx<'a> {
    fn focused_marker(&self) -> bool {
        self.tools.0
    }
}

/* =============================================================== App parts */

impl App {
    pub fn apply_style(&mut self, ctx: &egui::Context) {
        if self.style_sig.is_none() && self.s.accent_auto {
            if let Some(c) = accent_color() {
                self.s.accent = c;
            }
        }
        let dark = match self.s.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => ctx.input(|i| i.raw.system_theme) != Some(egui::Theme::Light),
        };
        let sig = (dark, self.s.accent, (self.s.text_scale * 100.0) as u32);
        if self.style_sig == Some(sig) {
            return;
        }
        self.style_sig = Some(sig);
        let ac = accent(&self.s);
        let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        if dark {
            v.panel_fill = Color32::from_rgb(32, 32, 32);
            v.window_fill = Color32::from_rgb(43, 43, 43);
            v.extreme_bg_color = Color32::from_rgb(26, 26, 26);
            v.faint_bg_color = Color32::from_rgb(40, 40, 40);
        } else {
            v.panel_fill = Color32::from_rgb(243, 243, 243);
            v.window_fill = Color32::from_rgb(252, 252, 252);
            v.extreme_bg_color = Color32::WHITE;
            v.faint_bg_color = Color32::from_rgb(249, 249, 249);
        }
        v.selection.bg_fill = ac.gamma_multiply(if dark { 0.55 } else { 0.85 });
        v.selection.stroke = Stroke::new(1.0, if dark { Color32::WHITE } else { Color32::BLACK });
        v.hyperlink_color = ac;
        let r = CornerRadius::same(4);
        v.widgets.noninteractive.corner_radius = r;
        v.widgets.inactive.corner_radius = r;
        v.widgets.hovered.corner_radius = r;
        v.widgets.active.corner_radius = r;
        v.widgets.open.corner_radius = r;
        v.window_corner_radius = CornerRadius::same(8);
        v.menu_corner_radius = CornerRadius::same(8);
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        ctx.set_visuals(v);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = vec2(6.0, 4.0);
            s.spacing.button_padding = vec2(8.0, 4.0);
            s.interaction.selectable_labels = false;
        });
        ctx.set_zoom_factor(self.s.text_scale);
    }

    pub fn is_dark(&self, ctx: &egui::Context) -> bool {
        ctx.style().visuals.dark_mode
    }

    /* ------------------------------------------------------------ keyboard */

    pub fn keyboard(&mut self, ctx: &egui::Context) {
        let modal = self.palette.is_some() || !matches!(self.dialog, Dialog::None);
        if modal {
            return;
        }
        let p = self.fp();
        let typing = ctx.wants_keyboard_input();
        let key = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        if key(Modifiers::COMMAND, Key::K) {
            self.acts.push(Act::Palette);
        }
        if key(cs, Key::F) {
            self.acts.push(Act::Dialog(Dialog::AdvSearch(AdvState::default())));
        }
        if key(Modifiers::COMMAND, Key::F) {
            self.focus_search = true;
        }
        if key(Modifiers::COMMAND, Key::L) {
            self.acts.push(Act::FocusAddress(p));
        }
        if key(Modifiers::COMMAND, Key::T) {
            let loc = self.panes[p].tab().loc.clone();
            self.acts.push(Act::NewTab(loc));
        }
        if key(Modifiers::COMMAND, Key::W) {
            let a = self.panes[p].active;
            self.acts.push(Act::CloseTab(p, a));
        }
        if !typing && key(cs, Key::N) {
            self.acts.push(Act::NewItem(p, true));
        }
        if key(Modifiers::COMMAND, Key::N) {
            let d = self.panes[p].tab().cur_dir().map(|d| d.to_path_buf());
            self.acts.push(Act::NewWindow(d));
        }
        if key(Modifiers::COMMAND, Key::Comma) {
            self.acts.push(Act::Dialog(Dialog::Settings(0)));
        }
        if key(Modifiers::NONE, Key::F9) {
            self.acts.push(Act::ToggleDual);
        }
        if key(Modifiers::NONE, Key::F11) {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        if key(Modifiers::ALT, Key::P) {
            self.s.details_pane = !self.s.details_pane;
            self.dirty = true;
        }
        if key(Modifiers::ALT, Key::ArrowLeft) {
            self.acts.push(Act::Back(p));
        }
        if key(Modifiers::ALT, Key::ArrowRight) {
            self.acts.push(Act::Forward(p));
        }
        if key(Modifiers::ALT, Key::ArrowUp) {
            self.acts.push(Act::Up(p));
        }
        if key(Modifiers::NONE, Key::F5) {
            self.acts.push(Act::Refresh(p));
        }
        if key(Modifiers::COMMAND, Key::Num1) {
            self.acts.push(Act::SetMode(p, ViewMode::Details));
        }
        if key(Modifiers::COMMAND, Key::Num2) {
            self.acts.push(Act::SetMode(p, ViewMode::Medium));
        }
        if ctx.input(|i| i.pointer.button_pressed(PointerButton::Extra1)) {
            self.acts.push(Act::Back(p));
        }
        if ctx.input(|i| i.pointer.button_pressed(PointerButton::Extra2)) {
            self.acts.push(Act::Forward(p));
        }
        // ctrl + wheel cycles the view size
        let (ctrl, wheel) = ctx.input(|i| (i.modifiers.command, i.raw_scroll_delta.y));
        if ctrl && wheel != 0.0 && !typing {
            let order = [ViewMode::Details, ViewMode::Small, ViewMode::Medium, ViewMode::Large];
            let cur = order.iter().position(|m| *m == self.panes[p].tab().mode).unwrap_or(0) as i32;
            let next = (cur + if wheel > 0.0 { 1 } else { -1 }).clamp(0, 3) as usize;
            self.acts.push(Act::SetMode(p, order[next]));
        }
        if let Some(q) = &self.ql {
            if ctx.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Space)) {
                let _ = q;
                self.ql = None;
                return;
            }
        }
        if typing {
            return;
        }
        let sel = self.panes[p].tab().selected_paths();
        let (copy_ev, cut_ev, paste_ev) = ctx.input(|i| {
            (
                i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                i.events.iter().any(|e| matches!(e, egui::Event::Cut)),
                i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
            )
        });
        if copy_ev && !sel.is_empty() {
            self.acts.push(Act::Copy(sel.clone(), false));
        }
        if cut_ev && !sel.is_empty() {
            self.acts.push(Act::Copy(sel.clone(), true));
        }
        if paste_ev || key(Modifiers::COMMAND, Key::V) {
            self.acts.push(Act::Paste(p));
        }
        if key(Modifiers::COMMAND, Key::A) {
            self.acts.push(Act::SelectAll(p));
        }
        if key(Modifiers::COMMAND, Key::Z) {
            self.acts.push(Act::Undo);
        }
        if key(cs, Key::C) && !sel.is_empty() {
            self.acts.push(Act::CopyPath(sel.clone()));
        }
        if key(Modifiers::SHIFT, Key::Delete) && !sel.is_empty() {
            self.acts.push(Act::Delete(sel.clone(), true));
        }
        if key(Modifiers::NONE, Key::Delete) && !sel.is_empty() {
            self.acts.push(Act::Delete(sel.clone(), false));
        }
        if key(Modifiers::NONE, Key::F2) {
            match sel.len() {
                1 => self.acts.push(Act::Rename(p, sel[0].clone())),
                n if n > 1 => self.acts.push(Act::Batch(sel.clone())),
                _ => {}
            }
        }
        if key(Modifiers::ALT, Key::Enter) && !sel.is_empty() {
            self.acts.push(Act::Props(sel[0].clone()));
        }
        if key(Modifiers::NONE, Key::Enter) {
            for s in &sel {
                self.acts.push(Act::Open(p, s.clone()));
            }
        }
        if key(Modifiers::NONE, Key::Backspace) {
            self.acts.push(Act::Up(p));
        }
        if key(Modifiers::NONE, Key::Space) && sel.len() == 1 {
            self.acts.push(Act::QuickLook(sel[0].clone()));
        }
        if key(Modifiers::NONE, Key::Escape) {
            self.panes[p].tab_mut().sel.clear();
        }
        // arrow navigation
        let tab = self.panes[p].tab_mut();
        let cols = if tab.mode == ViewMode::Details { 1 } else { tab.cols.max(1) } as i32;
        let mut delta = 0i32;
        let ext = ctx.input(|i| i.modifiers.shift);
        let mods = if ext { Modifiers::SHIFT } else { Modifiers::NONE };
        if key(mods, Key::ArrowDown) {
            delta = cols;
        }
        if key(mods, Key::ArrowUp) {
            delta = -cols;
        }
        if cols > 1 && key(mods, Key::ArrowRight) {
            delta = 1;
        }
        if cols > 1 && key(mods, Key::ArrowLeft) {
            delta = -1;
        }
        if delta != 0 && !tab.view.is_empty() {
            let cur = tab.anchor.map(|a| a as i32).unwrap_or(-delta.signum().min(0) - if delta > 0 { 1 } else { 0 });
            let cur = if tab.sel.is_empty() { if delta > 0 { -1 } else { tab.view.len() as i32 } } else { cur.max(0) };
            let next = (cur + delta).clamp(0, tab.view.len() as i32 - 1) as usize;
            let path = tab.list()[tab.view[next]].path.clone();
            if ext {
                click_select(tab, next, Modifiers::SHIFT);
            } else {
                tab.sel.clear();
                tab.sel.insert(path.clone());
                tab.anchor = Some(next);
            }
            tab.scroll_to = Some(path);
        }
    }

    /* ------------------------------------------------------------ top bar */

    pub fn top_bar(&mut self, ctx: &egui::Context) {
        let fill = ctx.style().visuals.panel_fill;
        egui::TopBottomPanel::top("top").frame(egui::Frame::new().fill(fill).inner_margin(Margin::symmetric(8, 4))).show(ctx, |ui| {
            self.tabs_row(ui);
            ui.add_space(2.0);
            self.nav_row(ui);
            ui.add_space(2.0);
            self.action_row(ui);
        });
    }

    fn tabs_row(&mut self, ui: &mut Ui) {
        let p = self.fp();
        let ac = accent(&self.s);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let n = self.panes[p].tabs.len();
            for a in 0..n {
                let title = self.panes[p].tabs[a].loc.title();
                let active = self.panes[p].active == a;
                let g = ui.painter().layout_no_wrap(title.clone(), FontId::proportional(13.0), ui.visuals().text_color());
                let w = (g.size().x + 54.0).clamp(96.0, 200.0);
                let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
                let bg = if active {
                    ui.visuals().widgets.noninteractive.weak_bg_fill.gamma_multiply(0.0).to_opaque()
                } else {
                    Color32::TRANSPARENT
                };
                let _ = bg;
                if active {
                    ui.painter().rect_filled(r, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, ui.visuals().extreme_bg_color);
                    ui.painter().rect_filled(Rect::from_min_size(pos2(r.min.x + 8.0, r.max.y - 2.5), vec2(w - 16.0, 2.5)), CornerRadius::same(2), ac);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(6), ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let ir = Rect::from_min_size(r.min + vec2(8.0, 6.0), vec2(16.0, 16.0));
                paint_folder(ui.painter(), ir, kind_color(Kind::Folder));
                let clip = ui.painter().with_clip_rect(Rect::from_min_max(r.min + vec2(28.0, 0.0), r.max - vec2(24.0, 0.0)));
                clip.text(r.left_center() + vec2(30.0, 0.0), Align2::LEFT_CENTER, &title, FontId::proportional(13.0), ui.visuals().text_color());
                let close_r = Rect::from_center_size(pos2(r.max.x - 13.0, r.center().y), vec2(18.0, 18.0));
                let close = ui.interact(close_r, ui.id().with(("tabx", a)), Sense::click());
                if n > 1 && (resp.hovered() || active) {
                    if close.hovered() {
                        ui.painter().rect_filled(close_r, CornerRadius::same(4), ui.visuals().widgets.hovered.bg_fill);
                    }
                    ui.painter().text(close_r.center(), Align2::CENTER_CENTER, "×", FontId::proportional(15.0), ui.visuals().text_color());
                }
                if close.clicked() && n > 1 {
                    self.acts.push(Act::CloseTab(p, a));
                } else if resp.clicked() {
                    self.acts.push(Act::SetTab(p, a));
                }
                if resp.middle_clicked() {
                    self.acts.push(Act::CloseTab(p, a));
                }
                resp.context_menu(|ui| {
                    if ui.button(t("Duplicate tab")).clicked() {
                        let l = self.panes[p].tabs[a].loc.clone();
                        self.acts.push(Act::NewTab(l));
                        ui.close_menu();
                    }
                    if n > 1 && ui.button(t("Close tab")).clicked() {
                        self.acts.push(Act::CloseTab(p, a));
                        ui.close_menu();
                    }
                    if n > 1 && ui.button(t("Close other tabs")).clicked() {
                        for k in (0..n).rev() {
                            if k != a {
                                self.acts.push(Act::CloseTab(p, k));
                            }
                        }
                        ui.close_menu();
                    }
                });
            }
            if ui.add(egui::Button::new(ic(Ic::Add)).frame(false)).on_hover_text(t("New tab")).clicked() {
                let loc = self.panes[p].tab().loc.clone();
                self.acts.push(Act::NewTab(loc));
            }
        });
    }

    fn nav_row(&mut self, ui: &mut Ui) {
        let p = self.fp();
        ui.horizontal(|ui| {
            let (can_back, can_fwd, can_up) = {
                let tb = self.panes[p].tab();
                (tb.hpos > 0, tb.hpos + 1 < tb.hist.len(), tb.loc != Loc::ThisPc)
            };
            let nb = |ui: &mut Ui, i: Ic, on: bool, tip: &str| ui.add_enabled(on, egui::Button::new(ic(i)).frame(false).min_size(vec2(30.0, 28.0))).on_hover_text(tip).clicked();
            if nb(ui, Ic::Back, can_back, t("Back")) {
                self.acts.push(Act::Back(p));
            }
            if nb(ui, Ic::Forward, can_fwd, t("Forward")) {
                self.acts.push(Act::Forward(p));
            }
            if nb(ui, Ic::Up, can_up, t("Up")) {
                self.acts.push(Act::Up(p));
            }
            if nb(ui, Ic::Refresh, true, t("Refresh")) {
                self.acts.push(Act::Refresh(p));
            }
            let total = ui.available_width();
            let search_w = 230.0;
            let addr_w = (total - search_w - 12.0).max(160.0);
            ui.allocate_ui_with_layout(vec2(addr_w, 30.0), Layout::left_to_right(Align::Center), |ui| self.address_bar(ui, p, addr_w));
            // search box
            let mut q = self.panes[p].tab().search.as_ref().map(|s| s.query.clone()).unwrap_or_else(|| self.panes[p].tab().filter.clone());
            let can_search = self.panes[p].tab().cur_dir().is_some();
            let id = egui::Id::new("search-box");
            let r = ui.add_enabled(
                can_search,
                egui::TextEdit::singleline(&mut q).id(id).hint_text(format!("{}  (Enter: {})", t("Filter"), t("Search").to_lowercase())).desired_width(search_w),
            );
            if self.focus_search {
                self.focus_search = false;
                r.request_focus();
            }
            if r.changed() {
                let tab = self.panes[p].tab_mut();
                if tab.search.is_some() {
                    tab.search = None;
                }
                tab.filter = q.clone();
                let s = &self.s;
                self.panes[p].tab_mut().rebuild(s);
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !q.trim().is_empty() {
                self.acts.push(Act::Search(p, q.clone()));
            }
            if (!q.is_empty() || self.panes[p].tab().search.is_some()) && ui.add(egui::Button::new(ic(Ic::Close)).frame(false)).clicked() {
                self.acts.push(Act::ClearSearch(p));
            }
        });
    }

    fn address_bar(&mut self, ui: &mut Ui, p: usize, width: f32) {
        let loc = self.panes[p].tab().loc.clone();
        let editing = self.panes[p].tab().addr_edit.is_some();
        egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .corner_radius(CornerRadius::same(5))
            .stroke(Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
            .inner_margin(Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.set_width(width - 14.0);
                ui.set_height(24.0);
                if editing {
                    let mut text = self.panes[p].tab().addr_edit.clone().unwrap_or_default();
                    let id = egui::Id::new(("addr", p));
                    let r = ui.add(egui::TextEdit::singleline(&mut text).id(id).desired_width(f32::INFINITY).frame(false));
                    if !r.has_focus() && !r.lost_focus() {
                        r.request_focus();
                    }
                    self.panes[p].tab_mut().addr_edit = Some(text.clone());
                    if r.lost_focus() {
                        if ui.input(|i| i.key_pressed(Key::Enter)) {
                            let path = PathBuf::from(text.trim().trim_matches('"'));
                            if path.is_dir() {
                                self.acts.push(Act::Navigate(p, Loc::Dir(path)));
                            } else {
                                self.err(format!("{}: {}", t("Cannot open this folder"), text));
                            }
                        }
                        self.panes[p].tab_mut().addr_edit = None;
                    }
                    return;
                }
                ui.spacing_mut().item_spacing.x = 0.0;
                let mut crumbs: Vec<(String, PathBuf)> = Vec::new();
                if let Loc::Dir(d) = &loc {
                    for a in d.ancestors() {
                        let label = a.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| a.to_string_lossy().into_owned());
                        crumbs.push((label, a.to_path_buf()));
                    }
                    crumbs.reverse();
                }
                // drop leading crumbs that do not fit
                let est = |c: &(String, PathBuf)| c.0.chars().count() as f32 * 7.4 + 34.0;
                let mut start = 0;
                let mut total: f32 = crumbs.iter().map(est).sum::<f32>() + 40.0;
                while total > width - 30.0 && start + 1 < crumbs.len() {
                    total -= est(&crumbs[start]);
                    start += 1;
                }
                if ui.add(egui::Button::new(ic(Ic::Home)).frame(false)).on_hover_text(t("This PC")).clicked() {
                    self.acts.push(Act::Navigate(p, Loc::ThisPc));
                }
                if start > 0 {
                    ui.label("…");
                }
                for (k, (label, path)) in crumbs.iter().enumerate().skip(start) {
                    let chev_id = ui.id().with(("chev", k));
                    let (cr, cresp) = ui.allocate_exact_size(vec2(16.0, 22.0), Sense::click());
                    paint_chevron(ui.painter(), cr.center(), 7.0, ui.visuals().weak_text_color(), false);
                    if cresp.clicked() {
                        let mut subs: Vec<(String, PathBuf)> = std::fs::read_dir(path)
                            .map(|rd| rd.flatten().filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)).map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect())
                            .unwrap_or_default();
                        subs.sort_by(|a, b| natural_cmp(&a.0.to_lowercase(), &b.0.to_lowercase()));
                        subs.truncate(300);
                        ui.data_mut(|d| d.insert_temp(chev_id, subs));
                        ui.memory_mut(|m| m.toggle_popup(chev_id));
                    }
                    egui::popup::popup_below_widget(ui, chev_id, &cresp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                        ui.set_min_width(180.0);
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            let subs: Vec<(String, PathBuf)> = ui.data(|d| d.get_temp(chev_id)).unwrap_or_default();
                            if subs.is_empty() {
                                ui.label(RichText::new(t("This folder is empty")).weak());
                            }
                            for (n, sp) in subs {
                                if ui.selectable_label(false, n).clicked() {
                                    self.acts.push(Act::Navigate(p, Loc::Dir(sp)));
                                }
                            }
                        });
                    });
                    let b = ui.add(egui::Button::new(label).frame(false));
                    if b.clicked() {
                        self.acts.push(Act::Navigate(p, Loc::Dir(path.clone())));
                    }
                    if let Some(pay) = b.dnd_release_payload::<Vec<PathBuf>>() {
                        let copy = ui.input(|i| i.modifiers.ctrl);
                        self.acts.push(Act::Drop { paths: (*pay).clone(), dest: path.clone(), copy });
                    }
                }
                let rest = ui.available_width().max(8.0);
                let (_, r) = ui.allocate_exact_size(vec2(rest, 22.0), Sense::click());
                if r.clicked() {
                    self.acts.push(Act::FocusAddress(p));
                }
                if let Loc::ThisPc = loc {
                    ui.painter().text(r.rect.left_center(), Align2::LEFT_CENTER, t("This PC"), FontId::proportional(13.5), ui.visuals().text_color());
                }
            });
    }

    fn action_row(&mut self, ui: &mut Ui) {
        let p = self.fp();
        let has_dir = self.panes[p].tab().cur_dir().is_some();
        let sel = self.panes[p].tab().selected_paths();
        let ac = accent(&self.s);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.add_enabled_ui(has_dir, |ui| {
                ui.menu_button(RichText::new(format!("{}  {}", ic(Ic::Add).text(), t("New"))).color(ac), |ui| {
                    if mi(ui, t("New folder")) {
                        self.acts.push(Act::NewItem(p, true));
                    }
                    if mi(ui, t("New file")) {
                        self.acts.push(Act::NewItem(p, false));
                    }
                });
            });
            ui.separator();
            let ib = |ui: &mut Ui, i: Ic, on: bool, tip: &str| ui.add_enabled(on, egui::Button::new(ic(i)).frame(false).min_size(vec2(28.0, 26.0))).on_hover_text(tip).clicked();
            if ib(ui, Ic::Cut, !sel.is_empty(), t("Cut")) {
                self.acts.push(Act::Copy(sel.clone(), true));
            }
            if ib(ui, Ic::Copy, !sel.is_empty(), t("Copy")) {
                self.acts.push(Act::Copy(sel.clone(), false));
            }
            if ib(ui, Ic::Paste, self.clip.is_some() && has_dir, t("Paste")) {
                self.acts.push(Act::Paste(p));
            }
            if ib(ui, Ic::Rename, sel.len() == 1, t("Rename")) {
                self.acts.push(Act::Rename(p, sel[0].clone()));
            }
            if ib(ui, Ic::Delete, !sel.is_empty(), t("Delete")) {
                self.acts.push(Act::Delete(sel.clone(), false));
            }
            if ib(ui, Ic::Undo, !self.undo.is_empty(), t("Undo")) {
                self.acts.push(Act::Undo);
            }
            ui.separator();
            let (key, asc, mode) = {
                let tb = self.panes[p].tab();
                (tb.key, tb.asc, tb.mode)
            };
            ui.menu_button(format!("{}  {}", ic(Ic::Sort).text(), t("Sort")), |ui| {
                for (k, l) in [(SortKey::Name, t("Name")), (SortKey::Modified, t("Date modified")), (SortKey::Kind, t("Type")), (SortKey::Size, t("Size"))] {
                    if ui.radio(key == k, l).clicked() {
                        if key != k {
                            self.acts.push(Act::Sort(p, k));
                        }
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.radio(asc, t("Ascending")).clicked() && !asc {
                    self.acts.push(Act::Sort(p, key));
                    ui.close_menu();
                }
                if ui.radio(!asc, t("Descending")).clicked() && asc {
                    self.acts.push(Act::Sort(p, key));
                    ui.close_menu();
                }
            });
            ui.menu_button(format!("{}  {}", ic(Ic::View).text(), t("View")), |ui| {
                for (m, l) in [(ViewMode::Details, t("Details")), (ViewMode::Small, t("Small icons")), (ViewMode::Medium, t("Medium icons")), (ViewMode::Large, t("Large icons"))] {
                    if ui.radio(mode == m, l).clicked() {
                        self.acts.push(Act::SetMode(p, m));
                        ui.close_menu();
                    }
                }
                ui.separator();
                let mut ch = false;
                ch |= ui.checkbox(&mut self.s.show_hidden, t("Show hidden files")).changed();
                ch |= ui.checkbox(&mut self.s.show_ext, t("File extensions")).changed();
                ch |= ui.checkbox(&mut self.s.compact, t("Compact rows")).changed();
                ch |= ui.checkbox(&mut self.s.details_pane, t("Details pane")).changed();
                if ch {
                    self.dirty = true;
                    for pi in 0..self.panes.len() {
                        for a in 0..self.panes[pi].tabs.len() {
                            let s = &self.s;
                            self.panes[pi].tabs[a].rebuild(s);
                        }
                    }
                }
            });
            // type filter
            let kf = self.panes[p].tab().kind_filter;
            let kf_label = kf.map(|k| t(kind_label(k))).unwrap_or(t("All types"));
            ui.menu_button(format!("{}: {}", t("Filter"), kf_label), |ui| {
                let opts: [(Option<Kind>, &str); 8] = [
                    (None, t("All types")),
                    (Some(Kind::Folder), t("Folders")),
                    (Some(Kind::Image), t("Images")),
                    (Some(Kind::Video), t("Videos")),
                    (Some(Kind::Audio), t("Audio")),
                    (Some(Kind::Document), t("Documents")),
                    (Some(Kind::Archive), t("Archives")),
                    (Some(Kind::Code), t("Code")),
                ];
                for (k, l) in opts {
                    if ui.radio(kf == k, l).clicked() {
                        let s = &self.s;
                        let tb = self.panes[p].tab_mut();
                        tb.kind_filter = k;
                        tb.rebuild(s);
                        ui.close_menu();
                    }
                }
            });
            ui.separator();
            if ui.add(egui::Button::new(ic(Ic::Info)).selected(self.s.details_pane).frame(self.s.details_pane)).on_hover_text(t("Details pane")).clicked() {
                self.s.details_pane = !self.s.details_pane;
                self.dirty = true;
            }
            if ui.add(egui::Button::new(ic(Ic::Dual)).selected(self.panes.len() > 1).frame(self.panes.len() > 1)).on_hover_text(format!("{} (F9)", t("Dual pane"))).clicked() {
                self.acts.push(Act::ToggleDual);
            }
            ui.menu_button(format!("{}  {}", ic(Ic::Tools).text(), t("Tools")), |ui| {
                if let Some(d) = self.panes[p].tab().cur_dir().map(|d| d.to_path_buf()) {
                    if mi(ui, t("Duplicate finder")) {
                        self.acts.push(Act::Dialog(Dialog::Dupes(DupState { root: d.clone(), min_kb: "1".into(), scanning: false, groups: vec![], picked: HashSet::new(), scanned: false })));
                    }
                    if mi(ui, t("Disk usage")) {
                        self.acts.push(Act::Usage(d.clone()));
                    }
                    if mi(ui, t("Terminal here")) {
                        self.acts.push(Act::OpenWith("terminal", d));
                    }
                }
                if mi(ui, t("Advanced search")) {
                    self.acts.push(Act::Dialog(Dialog::AdvSearch(AdvState::default())));
                }
                if mi(ui, t("Clean temp files")) {
                    self.acts.push(Act::CleanTemp);
                }
                if cfg!(windows) {
                    ui.separator();
                    ui.label(RichText::new(t("System tools")).weak().small());
                    for (l, k) in [
                        (t("Control Panel"), "control"),
                        (t("Windows Settings"), "settings"),
                        (t("Task Manager"), "taskmgr"),
                        (t("Device Manager"), "devmgmt"),
                        (t("Disk Management"), "diskmgmt"),
                        (t("Services"), "services"),
                        (t("Environment variables"), "envvars"),
                        (t("Registry Editor"), "regedit"),
                    ] {
                        if mi(ui, l) {
                            self.acts.push(Act::System(k));
                        }
                    }
                }
                ui.separator();
                if mi(ui, t("About")) {
                    self.acts.push(Act::Dialog(Dialog::About));
                }
            });
            if ui.add(egui::Button::new(ic(Ic::Settings)).frame(false)).on_hover_text(t("Settings")).clicked() {
                self.acts.push(Act::Dialog(Dialog::Settings(0)));
            }
        });
    }

    /* ----------------------------------------------------------- sidebar */

    pub fn sidebar(&mut self, ctx: &egui::Context) {
        let p = self.fp();
        let cur = self.panes[p].tab().loc.clone();
        let ac = accent(&self.s);
        let dark = self.is_dark(ctx);
        let fill = ctx.style().visuals.panel_fill;
        egui::SidePanel::left("side").resizable(true).default_width(232.0).width_range(160.0..=420.0).frame(egui::Frame::new().fill(fill).inner_margin(Margin::symmetric(6, 6))).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                // returns the response of a sidebar row
                let row = |ui: &mut Ui, selected: bool, label: &str, paint: &dyn Fn(&egui::Painter, Rect)| -> egui::Response {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                    if selected {
                        ui.painter().rect_filled(r, CornerRadius::same(5), ui.visuals().widgets.hovered.weak_bg_fill);
                        ui.painter().rect_filled(Rect::from_min_size(pos2(r.min.x, r.center().y - 8.0), vec2(3.0, 16.0)), CornerRadius::same(2), ac);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(5), ui.visuals().widgets.hovered.weak_bg_fill.gamma_multiply(0.7));
                    }
                    paint(ui.painter(), Rect::from_min_size(r.min + vec2(10.0, 6.0), vec2(18.0, 18.0)));
                    let clip = ui.painter().with_clip_rect(r.shrink2(vec2(2.0, 0.0)));
                    clip.text(r.left_center() + vec2(36.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(13.5), ui.visuals().text_color());
                    resp
                };
                let home_paint = |p: &egui::Painter, r: Rect| {
                    p.text(r.center(), Align2::CENTER_CENTER, ic(Ic::Home).text(), FontId::new(15.0, if has_icon_font() { egui::FontFamily::Name("icons".into()) } else { egui::FontFamily::Proportional }), ac);
                };
                if row(ui, cur == Loc::ThisPc, t("This PC"), &home_paint).clicked() {
                    self.acts.push(Act::Navigate(p, Loc::ThisPc));
                }
                ui.add_space(8.0);
                ui.label(RichText::new(t("Quick access")).weak().small());
                let mut items: Vec<(String, PathBuf, bool, usize)> = Vec::new(); // name, path, pinned, pin index
                for (n, d) in &self.places {
                    items.push((t(n).to_string(), d.clone(), false, 0));
                }
                for (i, pin) in self.s.pinned.iter().enumerate() {
                    let pb = PathBuf::from(pin);
                    let n = pb.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| pin.clone());
                    items.push((n, pb, true, i));
                }
                for (name, path, pinned, pi) in items {
                    let sel = cur == Loc::Dir(path.clone());
                    let col = self.s.folder_colors.get(path.to_string_lossy().as_ref()).map(|c| Color32::from_rgb(c[0], c[1], c[2])).unwrap_or(kind_color(Kind::Folder));
                    let resp = row(ui, sel, &name, &|p, r| paint_folder(p, r, col));
                    if resp.clicked() {
                        self.acts.push(Act::Navigate(p, Loc::Dir(path.clone())));
                    }
                    if resp.middle_clicked() {
                        self.acts.push(Act::NewTab(Loc::Dir(path.clone())));
                    }
                    if let Some(pay) = resp.dnd_release_payload::<Vec<PathBuf>>() {
                        let copy = ui.input(|i| i.modifiers.ctrl);
                        self.acts.push(Act::Drop { paths: (*pay).clone(), dest: path.clone(), copy });
                    }
                    resp.context_menu(|ui| {
                        if mi(ui, t("Open in new tab")) {
                            self.acts.push(Act::NewTab(Loc::Dir(path.clone())));
                        }
                        if pinned {
                            if pi > 0 && mi(ui, t("Move up")) {
                                self.s.pinned.swap(pi, pi - 1);
                                self.dirty = true;
                            }
                            if pi + 1 < self.s.pinned.len() && mi(ui, t("Move down")) {
                                self.s.pinned.swap(pi, pi + 1);
                                self.dirty = true;
                            }
                            if mi(ui, t("Unpin")) {
                                self.acts.push(Act::Pin(path.clone(), false));
                            }
                        } else if mi(ui, t("Pin to quick access")) {
                            self.acts.push(Act::Pin(path.clone(), true));
                        }
                    });
                }
                ui.add_space(8.0);
                ui.label(RichText::new(t("Drives")).weak().small());
                let drives = self.drives.clone();
                for d in drives {
                    let sel = cur == Loc::Dir(d.path.clone());
                    let label = if d.name.is_empty() { d.path.display().to_string() } else { format!("{} ({})", d.name, d.path.display()) };
                    let resp = row(ui, sel, &label, &|p, r| paint_drive(p, r, Color32::from_rgb(120, 130, 150)));
                    let used = d.total.saturating_sub(d.free) as f32 / d.total.max(1) as f32;
                    let bar = Rect::from_min_size(pos2(resp.rect.min.x + 36.0, resp.rect.max.y - 5.0), vec2(resp.rect.width() - 48.0, 3.0));
                    ui.painter().rect_filled(bar, CornerRadius::same(2), ui.visuals().widgets.inactive.bg_fill);
                    ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * used, 3.0)), CornerRadius::same(2), if used > 0.9 { Color32::from_rgb(196, 43, 28) } else { ac });
                    if resp.clicked() {
                        self.acts.push(Act::Navigate(p, Loc::Dir(d.path.clone())));
                    }
                    if let Some(pay) = resp.dnd_release_payload::<Vec<PathBuf>>() {
                        let copy = ui.input(|i| i.modifiers.ctrl);
                        self.acts.push(Act::Drop { paths: (*pay).clone(), dest: d.path.clone(), copy });
                    }
                }
                ui.add_space(8.0);
                ui.label(RichText::new(t("Tags")).weak().small());
                for i in 0..TAG_COLORS.len() {
                    let active = self.panes[p].tab().tag_filter == Some(i as u8);
                    let name = self.s.tag_names.get(i).cloned().unwrap_or_default();
                    let col = tag_color(i as u8);
                    let resp = row(ui, active, &name, &|p, r| {
                        p.circle_filled(r.center(), 5.0, col);
                    });
                    if resp.clicked() {
                        let s = &self.s;
                        let tb = self.panes[p].tab_mut();
                        tb.tag_filter = if active { None } else { Some(i as u8) };
                        tb.rebuild(s);
                    }
                }
                let _ = dark;
            });
        });
    }

    /* -------------------------------------------------------- status bar */

    pub fn status_bar(&mut self, ctx: &egui::Context) {
        let p = self.fp();
        let fill = ctx.style().visuals.panel_fill;
        let ac = accent(&self.s);
        egui::TopBottomPanel::bottom("status").frame(egui::Frame::new().fill(fill).inner_margin(Margin::symmetric(10, 3))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let tab = self.panes[p].tab();
                let mut txt = format!("{} {}", tab.view.len(), t("items"));
                if let Some(s) = &tab.search {
                    txt = format!("{} {}{}", s.results.len(), t("results"), if s.running { format!(" - {}", t("Searching...")) } else { String::new() });
                }
                if !tab.sel.is_empty() {
                    let sz: u64 = tab.selected_entries().iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
                    txt.push_str(&format!("   |   {} {}", tab.sel.len(), t("selected")));
                    if sz > 0 {
                        txt.push_str(&format!(" ({})", fmt_size(sz)));
                    }
                }
                ui.label(RichText::new(txt).small());
                if let Some(op) = &self.op {
                    ui.separator();
                    let done = op.shared.done.load(std::sync::atomic::Ordering::Relaxed);
                    let total = op.shared.total.load(std::sync::atomic::Ordering::Relaxed);
                    let name = op.shared.name.lock().map(|n| n.clone()).unwrap_or_default();
                    ui.label(RichText::new(&op.label).small());
                    if total > 0 {
                        ui.add(egui::ProgressBar::new((done as f32 / total as f32).clamp(0.0, 1.0)).desired_width(180.0).fill(ac).text(format!("{} / {}", fmt_size(done), fmt_size(total))));
                    } else {
                        ui.spinner();
                    }
                    ui.label(RichText::new(name).weak().small());
                    if ui.small_button(t("Cancel")).clicked() {
                        op.shared.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mode = self.panes[p].tab().mode;
                    for (m, g) in [(ViewMode::Large, "L"), (ViewMode::Medium, "M"), (ViewMode::Small, "S"), (ViewMode::Details, "☰")] {
                        if ui.add(egui::Button::new(g).selected(mode == m).frame(mode == m)).clicked() {
                            self.acts.push(Act::SetMode(p, m));
                        }
                    }
                });
            });
        });
    }

    /* ------------------------------------------------------ details pane */

    pub fn details_pane(&mut self, ctx: &egui::Context) {
        if !self.s.details_pane {
            return;
        }
        let p = self.fp();
        let dark = self.is_dark(ctx);
        let fill = ctx.style().visuals.panel_fill;
        let sel: Vec<Entry> = self.panes[p].tab().selected_entries().into_iter().cloned().collect();
        if let Some(e) = sel.first().filter(|e| sel.len() == 1 && is_text_ext(&e.ext)) {
            let path = e.path.clone();
            self.request_preview(&path, 4096);
        }
        let bus = self.bus();
        egui::SidePanel::right("details").resizable(true).default_width(270.0).frame(egui::Frame::new().fill(fill).inner_margin(Margin::same(12))).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if sel.is_empty() {
                    let tab = self.panes[p].tab();
                    ui.label(RichText::new(tab.loc.title()).heading());
                    ui.label(RichText::new(format!("{} {}", tab.view.len(), t("items"))).weak());
                    return;
                }
                if sel.len() > 1 {
                    let total: u64 = sel.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
                    ui.label(RichText::new(format!("{} {}", sel.len(), t("items selected"))).heading());
                    ui.label(RichText::new(fmt_size(total)).weak());
                    return;
                }
                let e = &sel[0];
                let side = ui.available_width().min(220.0);
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), side * 0.8), Sense::hover());
                let mut drew = false;
                if is_image_ext(&e.ext) {
                    if let Some(tex) = self.thumbs.tex.get(&e.path) {
                        let ts = tex.size_vec2();
                        let sc = (r.width() / ts.x).min(r.height() / ts.y).min(1.5);
                        ui.painter().image(tex.id(), Rect::from_center_size(r.center(), ts * sc), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        drew = true;
                    } else {
                        self.thumbs.request(&e.path, 256, &bus);
                    }
                }
                if !drew {
                    paint_entry_icon(ui.painter(), Rect::from_center_size(r.center(), vec2(side * 0.6, side * 0.6)), e, &self.s, dark);
                }
                ui.add_space(6.0);
                ui.label(RichText::new(&e.name).heading());
                ui.label(RichText::new(kind_text(e)).weak());
                ui.add_space(6.0);
                egui::Grid::new("dg").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    if !e.is_dir {
                        ui.label(RichText::new(t("Size")).weak());
                        ui.label(fmt_size(e.size));
                        ui.end_row();
                    }
                    ui.label(RichText::new(t("Modified")).weak());
                    ui.label(fmt_date(e.modified));
                    ui.end_row();
                    ui.label(RichText::new(t("Created")).weak());
                    ui.label(fmt_date(e.created));
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.label(RichText::new(e.path.display().to_string()).weak().small());
                if is_text_ext(&e.ext) {
                    ui.separator();
                    if let Some((pp, res)) = &self.preview {
                        if *pp == e.path {
                            match res {
                                Ok(txt) => {
                                    ui.label(RichText::new(txt).monospace().size(11.5));
                                }
                                Err(_) => {
                                    ui.label(RichText::new(t("Binary file")).weak());
                                }
                            }
                        }
                    }
                }
            });
        });
    }

    /* ------------------------------------------------------------ central */

    pub fn central(&mut self, ctx: &egui::Context) {
        let dark = self.is_dark(ctx);
        let fill = ctx.style().visuals.extreme_bg_color;
        let np = self.panes.len();
        let focus = self.fp();
        let tools = (false, false);
        let bus = self.bus();
        let App { panes, s, acts, thumbs, clip, drives, places, .. } = self;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(fill).inner_margin(Margin::symmetric(4, 0))).show(ctx, |ui| {
            let mut run = |ui: &mut Ui, i: usize| {
                let a = panes[i].active;
                let tab = &mut panes[i].tabs[a];
                let mut cx = PaneCx { s, acts, thumbs, clip, bus: bus.clone(), idx: i, dual: np > 1, dark, drives, places, tools: (focus == i, tools.1) };
                pane_ui(ui, tab, &mut cx);
            };
            if np == 1 {
                run(ui, 0);
            } else {
                ui.columns(2, |cols| {
                    for i in 0..2 {
                        run(&mut cols[i], i);
                    }
                });
            }
        });
    }

    /* ---------------------------------------------------------- overlays */

    pub fn overlays(&mut self, ctx: &egui::Context) {
        // external drag & drop (from Explorer into the app)
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if hovering {
            egui::Area::new(egui::Id::new("dropzone")).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).order(egui::Order::Tooltip).show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(Margin::same(18)).show(ui, |ui| {
                    ui.label(RichText::new(t("Drop files to copy them here")).heading());
                });
            });
        }
        if !dropped.is_empty() {
            let p = self.fp();
            if let Some(d) = self.panes[p].tab().cur_dir().map(|d| d.to_path_buf()) {
                self.paste_to(dropped, d, false);
            }
        }
        // drag preview
        if let Some(pay) = egui::DragAndDrop::payload::<Vec<PathBuf>>(ctx) {
            if let Some(pos) = ctx.pointer_latest_pos() {
                egui::Area::new(egui::Id::new("dragpreview")).fixed_pos(pos + vec2(14.0, 10.0)).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                        let n = pay.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.label(if pay.len() == 1 { n } else { format!("{} {}", pay.len(), t("items")) });
                    });
                });
            }
        }
        self.quick_look_ui(ctx);
        // toasts
        if !self.toasts.is_empty() {
            egui::Area::new(egui::Id::new("toasts")).anchor(Align2::RIGHT_BOTTOM, [-14.0, -42.0]).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
                for tst in &self.toasts {
                    let (bg, ic_) = match tst.kind {
                        ToastKind::Success => (Color32::from_rgb(16, 124, 16), "✓"),
                        ToastKind::Error => (Color32::from_rgb(196, 43, 28), "!"),
                        ToastKind::Info => (Color32::from_rgb(60, 70, 90), "i"),
                    };
                    egui::Frame::new().fill(bg).corner_radius(CornerRadius::same(6)).inner_margin(Margin::symmetric(12, 7)).show(ui, |ui| {
                        ui.label(RichText::new(format!("{}  {}", ic_, tst.text)).color(Color32::WHITE));
                    });
                    ui.add_space(4.0);
                }
            });
        }
    }

    fn quick_look_ui(&mut self, ctx: &egui::Context) {
        let Some(q) = self.ql.as_mut() else { return };
        let mut open = true;
        let title = q.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut open_default = false;
        egui::Window::new(RichText::new(format!("{} - {}", t("Quick look"), title)).strong()).open(&mut open).collapsible(false).resizable(true).default_size([760.0, 560.0]).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            if q.loading {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                });
                return;
            }
            if let Some(tex) = &q.tex {
                let avail = ui.available_size();
                let (rect, resp) = ui.allocate_exact_size(avail, Sense::drag());
                let ts = tex.size_vec2();
                let fit = (rect.width() / ts.x).min(rect.height() / ts.y).min(1.0);
                if resp.hovered() {
                    let d = ui.input(|i| i.raw_scroll_delta.y);
                    if d != 0.0 {
                        q.zoom = (q.zoom * (1.0 + d * 0.002)).clamp(0.1, 20.0);
                    }
                }
                if resp.dragged() {
                    q.pan += resp.drag_delta();
                }
                if resp.double_clicked() {
                    q.zoom = 1.0;
                    q.pan = egui::Vec2::ZERO;
                }
                let r = Rect::from_center_size(rect.center() + q.pan, ts * fit * q.zoom);
                ui.painter_at(rect).image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                match &q.text {
                    Some(Ok(txt)) => {
                        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                            ui.label(RichText::new(txt).monospace().size(12.5));
                        });
                    }
                    _ => {
                        ui.vertical_centered(|ui| {
                            ui.add_space(60.0);
                            ui.label(RichText::new(t("No preview available")).heading());
                            ui.add_space(8.0);
                            if ui.button(t("Open with default app")).clicked() {
                                open_default = true;
                            }
                        });
                    }
                }
            }
        });
        if open_default {
            let _ = open::that(&q.path);
        }
        if !open {
            self.ql = None;
        }
    }

    /* ---------------------------------------------------------------- acts */

    pub fn run_acts(&mut self, ctx: &egui::Context) {
        let acts = std::mem::take(&mut self.acts);
        for a in acts {
            match a {
                Act::Navigate(p, loc) => self.navigate(p, loc, true),
                Act::Open(p, path) => {
                    if path.is_dir() {
                        self.navigate(p, Loc::Dir(path), true);
                    } else if let Err(e) = open::that(&path) {
                        self.err(e.to_string());
                    }
                }
                Act::NewTab(loc) => {
                    let p = self.fp();
                    self.new_tab(p, loc);
                }
                Act::OpenOtherPane(path) => {
                    if self.panes.len() > 1 {
                        let other = 1 - self.fp();
                        self.focus = other;
                        self.navigate(other, Loc::Dir(path), true);
                    }
                }
                Act::NewWindow(path) => {
                    if let Ok(exe) = std::env::current_exe() {
                        let mut c = std::process::Command::new(exe);
                        if let Some(p) = path {
                            c.arg(p);
                        }
                        let _ = c.spawn();
                    }
                }
                Act::Back(p) => self.history_move(p, -1),
                Act::Forward(p) => self.history_move(p, 1),
                Act::Up(p) => self.go_up(p),
                Act::Refresh(p) => {
                    let a = self.panes[p].active;
                    self.load_tab(p, a);
                    self.drives = get_drives();
                }
                Act::Copy(paths, cut) => {
                    // also put the paths on the system clipboard as text; this makes the OS deliver a
                    // paste event for Ctrl+V even when the clipboard held nothing else
                    ctx.copy_text(paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
                    self.clip = Some(Clip { paths, cut });
                }
                Act::Usage(path) => {
                    self.dialog = Dialog::Usage(UsageState { path: path.clone(), loading: true, items: vec![], error: None, stack: vec![] });
                    let bus = self.bus();
                    std::thread::spawn(move || {
                        let never = std::sync::atomic::AtomicBool::new(false);
                        let res = folder_sizes(&path, &never);
                        bus.send(Msg::Sizes { path, res });
                    });
                }
                Act::Paste(p) => self.paste_into(p),
                Act::Delete(paths, perm) => {
                    if perm || self.s.confirm_delete {
                        self.dialog = Dialog::Delete { paths, perm };
                    } else {
                        self.start_delete(paths, false);
                    }
                }
                Act::Rename(p, path) => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.panes[p].tab_mut().rename = Some(RenameState { path: path.clone(), text: name, first: true });
                    self.panes[p].tab_mut().scroll_to = Some(path);
                }
                Act::CommitRename(p) => self.commit_rename(p),
                Act::CancelRename(p) => self.panes[p].tab_mut().rename = None,
                Act::NewItem(p, is_dir) => {
                    if let Some(dir) = self.panes[p].tab().cur_dir().map(|d| d.to_path_buf()) {
                        let name = if is_dir { t("New folder") } else { t("New text document.txt") };
                        match create_item(&dir, name, is_dir) {
                            Ok(path) => {
                                self.undo.push(Undo::Create { paths: vec![path.clone()] });
                                let tab = self.panes[p].tab_mut();
                                tab.rename_after = Some(path);
                                let a = self.panes[p].active;
                                self.load_tab(p, a);
                            }
                            Err(e) => self.err(App::friendly(&e)),
                        }
                    }
                }
                Act::Props(path) => {
                    self.dialog = Dialog::Props { path: path.clone(), props: None };
                    let bus = self.bus();
                    std::thread::spawn(move || {
                        let res = item_properties(&path);
                        bus.send(Msg::Props { path, res });
                    });
                }
                Act::Batch(paths) => self.dialog = Dialog::Batch(BatchState::new(paths)),
                Act::Compress(paths) => {
                    if let Some(first) = paths.first() {
                        let dest = first.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                        let name = if paths.len() == 1 { split_name(&first.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), first.is_dir()).0 } else { "Archive".into() };
                        self.dialog = Dialog::Compress { paths, dest, name, kind: 0 };
                    }
                }
                Act::Extract(path, to_folder) => {
                    let dest = path.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                    let bus = self.bus();
                    self.info(t("Extract here"));
                    std::thread::spawn(move || {
                        let res = extract_archive(&path, &dest, to_folder);
                        bus.send(Msg::Archive { res, extracted: true });
                    });
                }
                Act::OpenWith(tool, path) => {
                    if let Err(e) = open_with(tool, &path) {
                        self.err(App::friendly(&e));
                    }
                }
                Act::Reveal(path) => {
                    let _ = reveal_item(&path);
                }
                Act::CopyPath(paths) => {
                    let txt = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
                    ctx.copy_text(txt);
                    self.info(t("Path copied"));
                }
                Act::Pin(path, pin) => {
                    let s = path.to_string_lossy().into_owned();
                    self.s.pinned.retain(|x| *x != s);
                    if pin {
                        self.s.pinned.push(s);
                    }
                    self.dirty = true;
                }
                Act::Star(paths) => {
                    let all = paths.iter().all(|p| self.s.starred(p));
                    for p in paths {
                        let k = p.to_string_lossy().into_owned();
                        if all {
                            self.s.stars.remove(&k);
                        } else {
                            self.s.stars.insert(k);
                        }
                    }
                    self.dirty = true;
                }
                Act::Tag(paths, tag) => {
                    for p in paths {
                        let k = p.to_string_lossy().into_owned();
                        match tag {
                            Some(v) => {
                                self.s.tags.insert(k, v);
                            }
                            None => {
                                self.s.tags.remove(&k);
                            }
                        }
                    }
                    self.dirty = true;
                }
                Act::FolderColor(path, c) => {
                    let k = path.to_string_lossy().into_owned();
                    match c {
                        Some(c) => {
                            self.s.folder_colors.insert(k, c);
                        }
                        None => {
                            self.s.folder_colors.remove(&k);
                        }
                    }
                    self.dirty = true;
                }
                Act::Wallpaper(path) => match set_wallpaper(&path) {
                    Ok(()) => self.ok(t("Wallpaper changed")),
                    Err(e) => self.err(App::friendly(&e)),
                },
                Act::Custom(i, path) => {
                    if let Some(c) = self.s.custom_cmds.get(i).cloned() {
                        let dir = if path.is_dir() { path.clone() } else { path.parent().map(|d| d.to_path_buf()).unwrap_or_default() };
                        let args: Vec<String> = c.args.split_whitespace().map(|a| a.replace("{path}", &path.to_string_lossy()).replace("{dir}", &dir.to_string_lossy())).collect();
                        if let Err(e) = run_custom(&c.program, &args) {
                            self.err(App::friendly(&e));
                        }
                    }
                }
                Act::Drop { paths, dest, copy } => {
                    let same = paths.iter().all(|p| p.parent() == Some(dest.as_path()));
                    if !(same && !copy) {
                        self.paste_to(paths, dest, !copy);
                    }
                }
                Act::Focus(p) => self.focus = p,
                Act::QuickLook(path) => self.open_quick_look(path),
                Act::Undo => self.do_undo(),
                Act::CloseTab(p, a) => self.close_tab(p, a),
                Act::SetTab(p, a) => {
                    self.panes[p].active = a;
                    self.focus = p;
                }
                Act::MoveTab(..) => {}
                Act::Search(p, q) => self.start_search(p, q, SearchOpts::default()),
                Act::ClearSearch(p) => {
                    let s = &self.s;
                    let tab = self.panes[p].tab_mut();
                    tab.search = None;
                    tab.filter.clear();
                    tab.rebuild(s);
                }
                Act::SelectAll(p) => {
                    let tab = self.panes[p].tab_mut();
                    tab.sel = tab.view.iter().map(|&i| tab.list()[i].path.clone()).collect();
                }
                Act::Sort(p, key) => {
                    let s = &self.s;
                    let tab = self.panes[p].tab_mut();
                    if tab.key == key {
                        tab.asc = !tab.asc;
                    } else {
                        tab.key = key;
                        tab.asc = true;
                    }
                    tab.rebuild(s);
                    self.remember_folder_view(p);
                }
                Act::SetMode(p, m) => {
                    self.panes[p].tab_mut().mode = m;
                    self.s.mode = m;
                    self.remember_folder_view(p);
                }
                Act::ToggleDual => {
                    if self.panes.len() > 1 {
                        self.panes.truncate(1);
                        self.focus = 0;
                    } else {
                        self.add_second_pane();
                    }
                    self.dirty = true;
                }
                Act::Dialog(d) => self.dialog = d,
                Act::System(k) => {
                    if let Err(e) = run_system(k) {
                        self.err(App::friendly(&e));
                    }
                }
                Act::CleanTemp => {
                    let bus = self.bus();
                    std::thread::spawn(move || {
                        let r = clean_temp();
                        bus.send(Msg::Cleaned(r));
                    });
                }
                Act::Palette => {
                    self.palette = Some(PaletteState { query: String::new(), sel: 0, focus_set: false });
                }
                Act::FocusSearch => self.focus_search = true,
                Act::FocusAddress(p) => {
                    let tab = self.panes[p].tab_mut();
                    let text = tab.cur_dir().map(|d| d.display().to_string()).unwrap_or_default();
                    tab.addr_edit = Some(text);
                }
            }
        }
    }
}
