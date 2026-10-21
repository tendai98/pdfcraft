//! The phone layout (`compact.rs`): a top bar and a bottom mode bar instead of the desktop's tab
//! strip, mode bar and rail; panels as a sheet; and what the system Back button does.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::compact::{self, Back};

const FORM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/form.pdf");

/// A phone-sized window in the compact layout, optionally with a document open.
fn phone(open: bool) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(392.0, 850.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.set_option("ui-layout", "compact").unwrap();
        if open {
            app.open_bytes("form.pdf", Some(FORM.into()), std::fs::read(FORM).unwrap()).unwrap();
        }
        app
    });
    h.run_steps(8);
    h
}

#[test]
fn a_phone_gets_the_compact_chrome_and_a_desktop_window_keeps_its_own() {
    let h = phone(false);
    assert!(h.state().compact);
    for mode in ["All tools", "Read", "Edit", "Convert", "E-Sign"] {
        h.get_by_label(mode);
    }
    h.get_by_label("Find tools and commands");
    h.get_by_label("Menu");
    assert!(h.query_by_label("Discord").is_none(), "the desktop strip's community button is not on a phone");
    assert!(!h.state().left_open, "a phone starts with Home in full, not under the tools list");

    // Auto keeps the desktop layout off a touch platform, whatever the window's size.
    let mut desktop = Harness::builder().with_size(egui::vec2(392.0, 850.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app
    });
    desktop.run_steps(6);
    assert_eq!(desktop.state().compact, compact::TOUCH_PLATFORM);
}

#[test]
fn the_current_mode_toggles_its_panel_sheet() {
    let mut h = phone(true);
    assert!(!h.state().left_open);
    h.get_by_label("Edit").click();
    h.run_steps(4);
    assert!(h.state().left_open, "a mode opens its tools");
    h.get_by_label("Edit").click();
    h.run_steps(4);
    assert!(!h.state().left_open, "the same mode again hides them");
}

#[test]
fn back_closes_what_is_on_top_then_leaves_the_document_then_the_app() {
    let mut h = phone(true);
    let ctx = h.ctx.clone();

    h.state_mut().set_option("dialog", "properties").unwrap();
    h.run_steps(2);
    assert_eq!(compact::back(h.state_mut(), &ctx), Back::Escape, "a dialog cancels as it does on Escape");
    h.state_mut().dialog = None;

    h.state_mut().set_option("panel", "pages").unwrap();
    h.run_steps(2);
    assert_eq!(compact::back(h.state_mut(), &ctx), Back::Done);
    assert!(h.state().right.is_none(), "Back closes the sheet first");
    assert!(h.state().active.is_some(), "and keeps the document");

    assert_eq!(compact::back(h.state_mut(), &ctx), Back::Done);
    assert!(h.state().active.is_none(), "then goes Home");
    assert_eq!(h.state().views.len(), 1, "without closing the document");

    assert_eq!(compact::back(h.state_mut(), &ctx), Back::Leave, "at Home the app goes to the background");
}

#[test]
fn the_shell_keeps_clear_of_the_status_bar_and_the_navigation_buttons() {
    let mut h = phone(true);
    // A phone's status bar (24 pt) and three-button navigation bar (48 pt).
    let (top, bottom) = (24.0, 48.0);
    h.input_mut().safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { left: 0.0, top, right: 0.0, bottom }));
    h.run_steps(6);
    let height = 850.0;
    let home = h.get_by_label("Home").rect();
    assert!(home.top() >= top, "the top bar starts below the status bar: {home:?}");
    for mode in ["All tools", "Read", "Edit", "Convert", "E-Sign"] {
        let rect = h.get_by_label(mode).rect();
        assert!(rect.bottom() <= height - bottom + 0.5, "{mode} ends above the navigation buttons: {rect:?}");
    }
    // The strips behind the system bars are the bars' colour (the light theme here), not the
    // near-black that shows where nothing is painted, so the system's dark icons stay readable.
    let image = h.render().unwrap();
    let scale = image.height() as f32 / height;
    for y in [top * 0.5, height - bottom * 0.5] {
        let px = image.get_pixel(image.width() / 2, (y * scale) as u32);
        assert!(px[0] > 200 && px[1] > 200 && px[2] > 200, "the strip at y={y} is painted: {px:?}");
    }
}
