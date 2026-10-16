//! PdfCraft for Android: the egui shell (`pdfcraft-ui-egui`) in a NativeActivity.
//!
//! `ffi` exports `android_main`, which [`run`]s eframe with [`shell::Shell`]: the shell's own
//! `PdfCraftApp`, given an [`host::AndroidHost`] for files and the clipboard. Everything that needs
//! a Java API is a method of `PdfCraftActivity` (android/), called through `bridge`. Build with
//! `cargo xtask android`; see docs/android/README.md.
//!
//! On every other platform the library is empty, apart from the event protocol and its tests.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
mod events;

#[cfg(target_os = "android")]
mod bridge;
#[cfg(target_os = "android")]
mod ffi;
#[cfg(target_os = "android")]
mod host;
#[cfg(target_os = "android")]
mod shell;

/// Start the app on android-activity's thread; returns when the activity's native side ends.
#[cfg(target_os = "android")]
fn run(app: android_activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_tag("pdfcraft").with_max_level(log::LevelFilter::Info));
    // Panics go to logcat ("panicked at ..."), where the smoke test looks for them; the
    // default hook writes to stderr, which Android discards.
    std::panic::set_hook(Box::new(|info| log::error!("{info}")));
    let files = app.internal_data_path();
    let renderer = shell::renderer(files.as_deref());
    log::info!("starting PdfCraft {} with {renderer:?}", env!("CARGO_PKG_VERSION"));
    let options =
        eframe::NativeOptions { android_app: Some(app.clone()), persistence_path: files.map(|d| d.join("app.ron")), renderer, ..Default::default() };
    if let Err(e) = eframe::run_native("PdfCraft", options, Box::new(move |cc| Ok(Box::new(shell::Shell::new(cc, app))))) {
        log::error!("PdfCraft couldn't start: {e}");
    }
}
