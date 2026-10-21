//! The phone layout: one column for screens whose shorter side is under [`COMPACT_WIDTH`] points.
//!
//! The desktop shell puts a tab strip, a mode bar, a docked tools panel, the page, a docked
//! inspector and the page rail side by side. On a phone that leaves the page a sliver and the
//! controls out of reach (docs/android/README.md, "Phone layout"). The compact layout keeps every
//! one of those pieces and arranges them for a narrow touch screen:
//!
//! - a top bar: Home, the open documents as one menu (the tab strip), Find, Open and the main menu;
//! - a bottom bar with the five modes (the mode bar), in thumb reach;
//! - a page bar above it while a document is open (the rail's page and zoom controls, and the side
//!   panels);
//! - the tools panel or the inspector as a sheet over the lower part of the page, which can be
//!   dragged taller or shorter and closes with its ✕.
//!
//! Only the chrome is new. The page, Home, the panels' contents and the dialogs are the shell's
//! own, so every feature reachable on the desktop is reachable here.

use egui::{Align, CornerRadius, Layout, Rect, Sense, Stroke, vec2};

use crate::theme::{self, Tokens};
use crate::{Dialog, Mode, PdfCraftApp, PropsTab, RightPanel, icons};

/// Screens whose shorter side is narrower than this (in points) get the compact layout on a touch
/// platform: Android's own phone/tablet line (`sw600dp`), so a phone stays compact when turned.
pub const COMPACT_WIDTH: f32 = 600.0;
/// The smallest comfortable touch target (Material's 48 dp, less egui's spacing).
pub const TOUCH: f32 = 44.0;
const TOP_BAR: f32 = 52.0;
const NAV_BAR: f32 = 60.0;
const PAGE_BAR: f32 = 46.0;
/// Touch slop for dragging the sheet's edge (egui's default is 5 points, made for a mouse).
const SHEET_GRAB: f32 = 16.0;

/// Which layout the shell uses. `Auto` is compact on a phone and the desktop layout everywhere else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutPreference {
    #[default]
    Auto,
    Desktop,
    Compact,
}

impl LayoutPreference {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "desktop" => Some(Self::Desktop),
            "compact" => Some(Self::Compact),
            _ => None,
        }
    }
}

/// Whether this build runs on a touch-first platform, where `Auto` may choose the compact layout.
pub const TOUCH_PLATFORM: bool = cfg!(target_os = "android");

/// Whether to lay the shell out compactly for a screen of `size` points.
pub fn wanted(preference: LayoutPreference, size: egui::Vec2, touch: bool) -> bool {
    match preference {
        LayoutPreference::Desktop => false,
        LayoutPreference::Compact => true,
        LayoutPreference::Auto => {
            let shorter = size.x.min(size.y);
            touch && shorter.is_finite() && shorter > 0.0 && shorter < COMPACT_WIDTH
        }
    }
}

/// What the sheet over the page shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sheet {
    Tools,
    Inspector,
}

/// The inspector wins: it is opened on purpose (menu, page bar), while the tools panel is what a
/// mode opens. Read mode has no tools panel, as on the desktop.
fn sheet(app: &PdfCraftApp) -> Option<Sheet> {
    if app.active.is_some() && app.right.is_some() {
        Some(Sheet::Inspector)
    } else if app.left_open && app.mode != Mode::Read {
        Some(Sheet::Tools)
    } else {
        None
    }
}

/// Called when the shell switches into the compact layout: a phone starts with the page (or Home)
/// in full, not under the All tools list the desktop opens by default. A tool already chosen (from
/// the command line, a restored session) keeps its panel.
pub fn entered(app: &mut PdfCraftApp) {
    if app.left == crate::LeftPanel::AllTools {
        app.left_open = false;
    }
}

/// A document shown here for the first time fits the page to the screen's width: a whole page on
/// a phone is too small to read. Later zooming is the reader's own.
fn fit_width_once(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let Some(view) = app.active.and_then(|i| app.views.get_mut(i)) else { return };
    let seen = egui::Id::new(("compact-fit-width", view.id));
    if ctx.data(|d| d.get_temp::<bool>(seen)).is_none() {
        ctx.data_mut(|d| d.insert_temp(seen, true));
        view.fit = crate::canvas::Fit::Width;
        view.goto = Some((view.current, 0.0));
    }
}

/// Lay out one frame of the shell. Dialogs, the palette and notices are drawn by the caller.
pub fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    // The docking workspace isn't drawn here, so nothing else takes its reveal requests.
    app.pending_panel_reveals.clear();
    // Behind the status and navigation bars (outside the safe area), in the bars' colour.
    let ctx = ui.ctx().clone();
    ctx.layer_painter(egui::LayerId::background()).rect_filled(ctx.viewport_rect(), 0.0, t.chrome);
    ui.style_mut().interaction.resize_grab_radius_side = SHEET_GRAB;

    fit_width_once(app, &ctx);
    top_bar(app, ui);
    nav_bar(app, ui);
    let open = sheet(app);
    if open.is_none() && app.active.is_some() {
        page_bar(app, ui);
    }
    if let Some(which) = open {
        let height = ui.available_height();
        egui::Panel::bottom("compact_sheet")
            .resizable(true)
            .default_size((height * 0.5).max(200.0))
            .size_range(160.0..=(height * 0.9).max(160.0))
            .frame(
                egui::Frame::NONE
                    .fill(t.panel)
                    .stroke(Stroke::new(1.0, t.divider))
                    .corner_radius(CornerRadius { nw: 14, ne: 14, sw: 0, se: 0 })
                    .inner_margin(egui::Margin { left: 0, right: 0, top: 10, bottom: 0 }),
            )
            .show(ui, |ui| {
                // The grab handle says the sheet can be dragged.
                let top = ui.max_rect().center_top();
                ui.painter().rect_filled(Rect::from_center_size(top + vec2(0.0, -4.0), vec2(36.0, 4.0)), 2.0, t.divider);
                match which {
                    Sheet::Tools => crate::panels::left_panel(app, ui),
                    Sheet::Inspector => crate::panels::right_panel(app, ui),
                }
            });
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.pasteboard)).show(ui, |ui| match app.active {
        None if app.combine_showing() => crate::combine_ui::page(app, ui),
        None => crate::home::show(app, ui),
        Some(index) => crate::canvas::document_area(app, index, ui),
    });
}

/// What the system Back button (or gesture) asks of the platform after [`back`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Back {
    /// Something is open on top (a dialog, a prompt, a menu, the palette, full screen): send it
    /// Escape, which closes each of them the way it already cancels.
    Escape,
    /// Back took a step in the layout (closed a sheet, or went from a document to Home).
    Done,
    /// Nothing is left to close: the platform puts the app in the background.
    Leave,
}

/// Android's Back: close what is on top, then the sheet, then go from a document to Home. Unsaved
/// work is never closed by Back; at Home the app only goes to the background.
pub fn back(app: &mut PdfCraftApp, ctx: &egui::Context) -> Back {
    if app.modal_open() || app.palette_open || app.full_screen || egui::Popup::is_any_open(ctx) {
        return Back::Escape;
    }
    match sheet(app) {
        Some(Sheet::Inspector) => {
            app.choose_right_panel(None);
            app.pending_panel_reveals.clear();
            Back::Done
        }
        Some(Sheet::Tools) => {
            app.left_open = false;
            Back::Done
        }
        None if app.active.is_some() || app.combine_showing() => {
            app.active = None;
            app.combine_tab.focused = false;
            Back::Done
        }
        None => Back::Leave,
    }
}

/// A square touch-sized icon button.
fn icon_button(ui: &mut egui::Ui, icon: &str, selected: bool, tip: &str) -> egui::Response {
    icons::button(ui, icon, TOUCH, selected, tip)
}

/// The name in the top bar: the active document, Combine, or Home.
fn title(app: &PdfCraftApp) -> String {
    if let Some(name) = app.active.and_then(|i| app.views.get(i)).and_then(|v| app.session.get(v.id)).map(|d| d.display_name()) {
        name
    } else if app.combine_showing() {
        tl!("Combine files").to_owned()
    } else {
        tl!("Home").to_owned()
    }
}

fn top_bar(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("compact_top_bar")
        .exact_size(TOP_BAR)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(4, 0)).stroke(Stroke::new(1.0, t.divider)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if icon_button(ui, "house", app.active.is_none() && !app.combine_showing(), tl!("Home")).clicked() {
                    app.active = None;
                    app.combine_tab.focused = false;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let menu = icon_button(ui, "ellipsis-vertical", false, tl!("Menu"));
                    egui::Popup::menu(&menu).show(|ui| overflow_menu(app, ui));
                    if icon_button(ui, "plus", false, tl!("Open")).clicked() {
                        app.open_dialog();
                    }
                    if icon_button(ui, "search", false, tl!("Find tools and commands")).clicked() {
                        app.palette_open = true;
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| document_switcher(app, ui));
                });
            });
        });
}

/// The open documents (the desktop's tab strip) as one button and a menu.
fn document_switcher(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let width = ui.available_width().max(1.0);
    let open = app.views.len() + usize::from(app.combine_tab.open);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, TOUCH), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("All open documents")));
    if resp.hovered() || resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect.shrink2(vec2(2.0, 4.0)), 8.0, t.hover);
    }
    let chevron = if open > 0 { 22.0 } else { 0.0 };
    let mut job = egui::text::LayoutJob::simple_singleline(crate::bidi::visual(&title(app)).into_owned(), theme::semibold(15.0), t.text);
    job.wrap =
        egui::text::TextWrapping { max_width: (rect.width() - 16.0 - chevron).max(1.0), max_rows: 1, break_anywhere: true, ..Default::default() };
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let text_pos = rect.left_center() + vec2(8.0, -galley.size().y * 0.5);
    let text_right = text_pos.x + galley.size().x;
    ui.painter().galley(text_pos, galley, t.text);
    if open > 0 {
        let icon = Rect::from_center_size(egui::pos2(text_right + 12.0, rect.center().y), vec2(16.0, 16.0));
        icons::paint(ui, icon, "chevron-down", 16.0, t.text_muted);
    }
    if open == 0 {
        return;
    }
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(260.0);
        let mut close = None;
        let mut activate = None;
        for (i, view) in app.views.iter().enumerate() {
            let Some(doc) = app.session.get(view.id) else { continue };
            let name = crate::bidi::visual(&doc.display_name()).into_owned();
            let unsaved = doc.dirty;
            ui.horizontal(|ui| {
                let label = if unsaved { format!("● {name}") } else { name };
                let row = ui.add_sized(
                    [(ui.available_width() - TOUCH).max(1.0), TOUCH],
                    egui::Button::selectable(app.active == Some(i) && !app.combine_showing(), label).truncate(),
                );
                if row.clicked() {
                    activate = Some(i);
                }
                if icons::button(ui, "x", TOUCH, false, tl!("Close Tab")).clicked() {
                    close = Some(view.id);
                }
            });
        }
        if app.combine_tab.open {
            ui.horizontal(|ui| {
                let row = ui.add_sized(
                    [(ui.available_width() - TOUCH).max(1.0), TOUCH],
                    egui::Button::selectable(app.combine_showing(), tl!("Combine files")).truncate(),
                );
                if row.clicked() {
                    app.open_combine_tab();
                    ui.close();
                }
                if icons::button(ui, "x", TOUCH, false, tl!("Close Tab")).clicked() {
                    app.close_combine_tab();
                    ui.close();
                }
            });
        }
        if let Some(i) = activate {
            app.active = Some(i);
            app.combine_tab.focused = false;
            ui.close();
        }
        if let Some(id) = close {
            app.tab_action(crate::tab_context::Action::Close(id));
            ui.close();
        }
    });
}

/// The ⋮ menu: the mode bar's document buttons, then the desktop's main menu.
fn overflow_menu(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    ui.set_min_width(240.0);
    let has_doc = app.active.is_some();
    ui.add_enabled_ui(has_doc, |ui| {
        if ui.add(egui::Button::new(tl!("Save")).min_size(vec2(0.0, 36.0))).clicked() {
            app.run_command("file.save");
            ui.close();
        }
        if ui.add(egui::Button::new(tl!("Document properties…")).min_size(vec2(0.0, 36.0))).clicked() {
            app.dialog = Some(Dialog::Properties(PropsTab::Description));
            ui.close();
        }
        if ui.add(egui::Button::new(tl!("Print")).min_size(vec2(0.0, 36.0))).clicked() {
            app.run_command("print.dialog");
            ui.close();
        }
    });
    ui.separator();
    crate::chrome::main_menu_body(app, ui);
}

/// The mode bar as a bottom navigation bar. Tapping the current mode shows or hides its panel.
fn nav_bar(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("compact_nav_bar").exact_size(NAV_BAR).frame(egui::Frame::NONE.fill(t.chrome).stroke(Stroke::new(1.0, t.divider))).show(
        ui,
        |ui| {
            const MODES: [(Mode, &str, &str); 5] = [
                (Mode::AllTools, "layout-grid", "All tools"),
                (Mode::Read, "book-open", "Read"),
                (Mode::Edit, "pencil", "Edit"),
                (Mode::Convert, "file-output", "Convert"),
                (Mode::Sign, "signature", "E-Sign"),
            ];
            let full = ui.max_rect();
            let cell = full.width() / MODES.len() as f32;
            for (n, (mode, icon, label)) in MODES.iter().enumerate() {
                let rect = Rect::from_min_size(full.min + vec2(cell * n as f32, 0.0), vec2(cell, full.height()));
                let resp = ui.interact(rect, ui.id().with(("mode", n)), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, app.mode == *mode, tl!(label)));
                let active = app.mode == *mode;
                let color = if active { t.accent } else { t.text_muted };
                let pill = Rect::from_center_size(rect.center_top() + vec2(0.0, 19.0), vec2(52.0, 28.0));
                if active {
                    ui.painter().rect_filled(pill, 14.0, t.selected);
                } else if resp.is_pointer_button_down_on() {
                    ui.painter().rect_filled(pill, 14.0, t.hover);
                }
                icons::paint(ui, Rect::from_center_size(pill.center(), vec2(20.0, 20.0)), icon, 20.0, color);
                let font = if active { theme::semibold(11.0) } else { theme::medium(11.0) };
                let mut job = egui::text::LayoutJob::simple_singleline(tl!(label).to_owned(), font, color);
                job.wrap = egui::text::TextWrapping { max_width: (cell - 4.0).max(1.0), max_rows: 1, break_anywhere: true, ..Default::default() };
                job.halign = Align::Center;
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                ui.painter().galley(rect.center_top() + vec2(0.0, 38.0), galley, color);
                if resp.clicked() {
                    tap_mode(app, *mode);
                }
            }
        },
    );
}

fn tap_mode(app: &mut PdfCraftApp, mode: Mode) {
    if app.mode == mode && mode != Mode::Read {
        // The panel is the mode's whole content here, so the same tab toggles it.
        let showing_tools = sheet(app) == Some(Sheet::Tools);
        app.left_open = !showing_tools;
        if !showing_tools {
            app.reveal_tools();
        }
    } else {
        app.switch_mode(mode, true);
    }
    if sheet(app) == Some(Sheet::Inspector) && app.left_open && mode != Mode::Read {
        // A mode's panel replaces the inspector in the one sheet there is room for.
        app.choose_right_panel(None);
    }
    app.pending_panel_reveals.clear();
}

/// The rail's page controls in one row: page, zoom, fit, and the side panels.
fn page_bar(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some((index, id)) = app.active_ids() else { return };
    let Some(doc) = app.session.get(id) else { return };
    let page_count = doc.info.pages.len();
    let mut chosen_panel: Option<Option<RightPanel>> = None;
    egui::Panel::bottom("compact_page_bar")
        .exact_size(PAGE_BAR)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(4, 0)).stroke(Stroke::new(1.0, t.divider)))
        .show(ui, |ui| {
            let Some(view) = app.views.get_mut(index) else { return };
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if icons::button(ui, "chevron-up", 40.0, false, tl!("Previous page")).clicked() {
                    view.step_page(false);
                }
                let page = format!("{} / {page_count}", view.current.saturating_add(1).min(page_count.max(1)));
                ui.allocate_ui_with_layout(vec2(64.0, 40.0), Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                    ui.label(egui::RichText::new(page).font(theme::medium(13.0)).color(t.text));
                });
                if icons::button(ui, "chevron-down", 40.0, false, tl!("Next page")).clicked() {
                    view.step_page(true);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let panels = icons::button(ui, "panel-right", 40.0, false, tl!("Side panels"));
                    egui::Popup::menu(&panels).show(|ui| {
                        ui.set_min_width(220.0);
                        for (p, label) in crate::chrome::SIDE_PANELS {
                            if ui.add(egui::Button::new(tl!(label)).min_size(vec2(0.0, 36.0))).clicked() {
                                chosen_panel = Some(Some(p));
                                ui.close();
                            }
                        }
                    });
                    if icons::button(ui, "zoom-in", 40.0, false, tl!("Zoom in")).clicked() {
                        view.zoom_step(true);
                    }
                    if icons::button(ui, "zoom-out", 40.0, false, tl!("Zoom out")).clicked() {
                        view.zoom_step(false);
                    }
                    let fit_icon = if view.fit == crate::canvas::Fit::Width { "maximize" } else { "maximize-2" };
                    if icons::button(ui, fit_icon, 40.0, false, tl!("Toggle fit page / fit width")).clicked() {
                        view.fit = if view.fit == crate::canvas::Fit::Width { crate::canvas::Fit::Page } else { crate::canvas::Fit::Width };
                        view.goto = Some((view.current, 0.0));
                    }
                    ui.label(egui::RichText::new(format!("{:.0}%", view.zoom * 100.0)).font(theme::medium(12.0)).color(t.text_muted));
                });
            });
        });
    if let Some(panel) = chosen_panel {
        app.choose_right_panel(panel);
        app.pending_panel_reveals.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_compact_only_on_a_touch_platform_with_a_phone_sized_shorter_side() {
        let phone = vec2(392.0, 850.0);
        assert!(wanted(LayoutPreference::Auto, phone, true));
        assert!(wanted(LayoutPreference::Auto, vec2(850.0, 392.0), true), "a phone turned sideways stays compact");
        assert!(!wanted(LayoutPreference::Auto, vec2(800.0, 1280.0), true), "tablets keep the desktop layout");
        assert!(!wanted(LayoutPreference::Auto, phone, false), "a narrow desktop window keeps the desktop layout");
        for bad in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert!(!wanted(LayoutPreference::Auto, vec2(bad, 800.0), true), "{bad}");
        }
    }

    #[test]
    fn an_explicit_preference_wins() {
        assert!(wanted(LayoutPreference::Compact, vec2(1440.0, 900.0), false));
        assert!(!wanted(LayoutPreference::Desktop, vec2(392.0, 850.0), true));
    }

    #[test]
    fn preferences_parse() {
        assert_eq!(LayoutPreference::parse("compact"), Some(LayoutPreference::Compact));
        assert_eq!(LayoutPreference::parse("desktop"), Some(LayoutPreference::Desktop));
        assert_eq!(LayoutPreference::parse("auto"), Some(LayoutPreference::Auto));
        assert_eq!(LayoutPreference::parse("phone"), None);
    }
}
