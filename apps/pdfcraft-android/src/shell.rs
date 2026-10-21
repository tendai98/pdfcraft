//! [`Shell`]: the shell's `PdfCraftApp` with what only Android needs around it. It answers the
//! activity's events, follows the system theme, shows the soft keyboard while a text field has
//! focus, and puts copied text on the system clipboard (egui-winit has no Android clipboard).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use android_activity::AndroidApp;
use pdfcraft_ui_egui::{PdfCraftApp, RecoveryStore, compact};

use crate::host::{AndroidHost, ForShell};

/// The renderer: wgpu (Vulkan, or GLES through wgpu), unless the app's files folder holds a
/// `renderer` file saying `gl`, which starts egui's OpenGL renderer instead (docs/android/devices.md).
pub(crate) fn renderer(files: Option<&Path>) -> eframe::Renderer {
    let choice = files.and_then(|d| std::fs::read_to_string(d.join("renderer")).ok()).unwrap_or_default();
    if choice.trim().eq_ignore_ascii_case("gl") { eframe::Renderer::Glow } else { eframe::Renderer::Wgpu }
}

pub(crate) struct Shell {
    app: PdfCraftApp,
    host: Arc<AndroidHost>,
    android: AndroidApp,
    keyboard_shown: bool,
    /// Whether the system bars were last set for a light app (`None`: not yet).
    light_bars: Option<bool>,
}

impl Shell {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, android: AndroidApp) -> Self {
        let mut app = PdfCraftApp::new();
        if let Some(json) = cc.storage.and_then(|s| s.get_string("pdfcraft")) {
            app.restore(&json);
        }
        let host = Arc::new(AndroidHost::new(android.clone()));
        app.host = host.clone();
        // Unsaved work is autosaved under filesDir (the activity points XDG_DATA_HOME there).
        if let Some(dir) = RecoveryStore::default_dir() {
            app.enable_recovery(RecoveryStore::new(dir));
        }
        if let Some(state) = &cc.wgpu_render_state {
            let adapter = state.adapter.get_info();
            log::info!("renderer: wgpu; adapter: {} ({:?}, {:?})", adapter.name, adapter.device_type, adapter.backend);
        } else {
            log::info!("renderer: OpenGL (glow)");
        }
        Shell { app, host, android, keyboard_shown: false, light_bars: None }
    }
}

impl eframe::App for Shell {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.system_theme = Some(if self.host.dark() { egui::Theme::Dark } else { egui::Theme::Light });
        // The window is drawn edge to edge: keep the shell out from under the status bar, the
        // navigation bar, a display cutout and the soft keyboard.
        raw.safe_area_insets = Some(safe_area(self.host.insets(), ctx.pixels_per_point()));
        // The Back button arrives as BrowserBack (winit); it is the shell's to interpret.
        let back = raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::BrowserBack, pressed: true, .. }));
        raw.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::BrowserBack, .. }));
        if back {
            match compact::back(&mut self.app, ctx) {
                compact::Back::Escape => {
                    for pressed in [true, false] {
                        raw.events.push(egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
                compact::Back::Done => ctx.request_repaint(),
                compact::Back::Leave => self.host.move_to_back(),
            }
        }
        self.app.raw_input_hook(ctx, raw);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app.save(storage);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        for event in self.host.poll() {
            match event {
                ForShell::Open(path) => self.app.open_path(&path.to_string_lossy()),
                ForShell::Notice(message) => self.app.notify(message),
            }
        }
        self.app.logic(ctx, frame);
        let wants = ctx.egui_wants_keyboard_input();
        if wants != self.keyboard_shown {
            if wants {
                self.android.show_soft_input(false);
            } else {
                self.android.hide_soft_input(false);
            }
            self.keyboard_shown = wants;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        let copied: Vec<String> = ui.ctx().output(|o| {
            o.commands
                .iter()
                .filter_map(|c| match c {
                    egui::OutputCommand::CopyText(text) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        });
        for text in copied {
            self.host.set_clipboard_text(&text);
        }
        // Status bar icons that show against the app's own theme, which may not be the system's.
        let light = !ui.style().visuals.dark_mode;
        if self.light_bars != Some(light) {
            self.host.set_light_bars(light);
            self.light_bars = Some(light);
        }
    }

    /// Android stops a backgrounded app without asking: keep settings at most this old.
    fn auto_save_interval(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Insets in pixels (left, top, right, bottom) as egui's safe area, in points.
fn safe_area(px: [u32; 4], pixels_per_point: f32) -> egui::SafeAreaInsets {
    let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
    let [left, top, right, bottom] = px.map(|v| v as f32 / ppp);
    egui::SafeAreaInsets(egui::epaint::MarginF32 { left, top, right, bottom })
}
