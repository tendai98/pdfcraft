//! [`AndroidHost`]: the shell's [`PlatformHost`] over the Storage Access Framework.
//!
//! Pickers are answered with copies of the chosen documents in the app's cache (`cacheDir/saf/`),
//! so the shell's path-based follow-ups run unchanged; a save picker answers with the path to
//! write, and a folder picker with an empty folder to write into. Whatever the shell then writes
//! there is published to the real document by the activity ([`PlatformHost::after_write`],
//! [`PlatformHost::after_pick`]). See docs/android/README.md.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Mutex, PoisonError, mpsc};
use std::time::SystemTime;

use android_activity::AndroidApp;
use pdfcraft_ui_egui::platform_host::{Dialog, PickFuture, PickKind, PlatformHost};

use crate::bridge::{self, Published};
use crate::events::{self, Event};

/// Files listed in a folder picked to write into, at most, when publishing it.
const MAX_FOLDER_FILES: usize = 10_000;
/// Events taken from the activity per frame, at most.
const MAX_EVENTS_PER_FRAME: usize = 64;

/// The size and modification time a file had when it was last published.
type Stamp = (u64, Option<SystemTime>);

pub struct AndroidHost {
    app: AndroidApp,
    next_id: AtomicI64,
    /// Pickers waiting for their `picked` event.
    pending: Mutex<HashMap<u64, mpsc::Sender<Vec<PathBuf>>>>,
    /// Files already published, so a save published by `after_write` isn't copied again by
    /// `after_pick` (Save As goes through both).
    published: Mutex<HashMap<PathBuf, Stamp>>,
    dark: AtomicBool,
    /// Messages for the user from calls that can't show one themselves.
    notices: Mutex<Vec<String>>,
}

/// What the shell does with an event (the host handles the rest itself).
pub enum ForShell {
    Open(PathBuf),
    Notice(String),
}

impl AndroidHost {
    pub fn new(app: AndroidApp) -> Self {
        let dark = bridge::is_dark_mode(&app).unwrap_or(false);
        AndroidHost {
            app,
            next_id: AtomicI64::new(1),
            pending: Mutex::default(),
            published: Mutex::default(),
            dark: AtomicBool::new(dark),
            notices: Mutex::default(),
        }
    }

    /// Whether the system is in dark mode, as last reported.
    pub fn dark(&self) -> bool {
        self.dark.load(Ordering::Relaxed)
    }

    fn notice(&self, message: impl Into<String>) {
        self.notices.lock().unwrap_or_else(PoisonError::into_inner).push(message.into());
    }

    /// Take the activity's events: answer pickers, follow the theme, and return what the shell
    /// must act on. Called once a frame.
    pub fn poll(&self) -> Vec<ForShell> {
        let mut out: Vec<ForShell> =
            std::mem::take(&mut *self.notices.lock().unwrap_or_else(PoisonError::into_inner)).into_iter().map(ForShell::Notice).collect();
        for _ in 0..MAX_EVENTS_PER_FRAME {
            let line = match bridge::poll_event(&self.app) {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(e) => {
                    log::warn!("couldn't read the activity's events: {e}");
                    break;
                }
            };
            match events::parse(&line) {
                Some(Event::Picked { id, paths }) => {
                    if let Some(tx) = self.pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&id) {
                        // The worker waiting for it may have gone (the app is closing): nothing to do.
                        let _ = tx.send(paths);
                    }
                }
                Some(Event::Open(path)) => out.push(ForShell::Open(path)),
                Some(Event::Theme { dark }) => self.dark.store(dark, Ordering::Relaxed),
                // Dropping rendered pages on memory pressure is Phase 3 (`trim_caches`).
                Some(Event::Trim { level }) => log::info!("onTrimMemory({level})"),
                Some(Event::Notice(m)) => out.push(ForShell::Notice(m)),
                None => log::warn!("ignored a malformed event from the activity"),
            }
        }
        out
    }

    pub fn set_clipboard_text(&self, text: &str) {
        if let Err(e) = bridge::set_clipboard_text(&self.app, text) {
            log::warn!("couldn't copy to the clipboard: {e}");
        }
    }

    /// Publish `path` unless it was published as it is now. `Ok(false)`: not a staged document.
    fn publish_once(&self, path: &Path) -> Result<bool, String> {
        let stamp = std::fs::metadata(path).ok().map(|m| (m.len(), m.modified().ok()));
        let mut published = self.published.lock().unwrap_or_else(PoisonError::into_inner);
        if stamp.is_some() && published.get(path) == stamp.as_ref() {
            return Ok(true);
        }
        let Some(p) = path.to_str() else { return Err("the file name isn't valid Unicode".into()) };
        match bridge::publish(&self.app, p)? {
            Published::Copied => {
                if let Some(stamp) = stamp {
                    published.insert(path.to_path_buf(), stamp);
                }
                Ok(true)
            }
            Published::NotStaged => Ok(false),
        }
    }
}

impl PlatformHost for AndroidHost {
    fn pick(&self, kind: PickKind, dialog: Dialog) -> PickFuture {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        // Waiting before asking, so the answer always finds its picker.
        let key = u64::try_from(id).unwrap_or_default();
        self.pending.lock().unwrap_or_else(PoisonError::into_inner).insert(key, tx);
        let asked = match kind {
            PickKind::File | PickKind::Files => {
                bridge::request_open(&self.app, id, &events::mime_types(&dialog.extensions()), kind == PickKind::Files)
            }
            PickKind::Save => {
                let name = dialog.file_name.unwrap_or_else(|| "document".to_string());
                bridge::request_save(&self.app, id, &name, events::mime_for_name(&name))
            }
            PickKind::Folder => bridge::request_folder(&self.app, id),
            PickKind::FolderToRead => Err("Choosing a folder to read isn't available on Android yet. Choose the files instead.".to_string()),
        };
        if let Err(e) = asked {
            // Dropping the sender ends the wait below with nothing chosen, as for a cancel.
            self.pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&key);
            self.notice(e);
        }
        // Awaited on the shell's picker worker thread, so blocking here blocks only that worker;
        // a closed channel (picker failed, or app closing) is a cancel.
        Box::pin(async move { rx.recv().unwrap_or_default() })
    }

    fn after_write(&self, path: &Path) -> Result<(), String> {
        self.publish_once(path).map(|_| ())
    }

    fn after_pick(&self, kind: PickKind, paths: &[PathBuf]) -> Vec<String> {
        let mut errors = Vec::new();
        match kind {
            PickKind::Save => {
                for p in paths.iter().filter(|p| p.is_file()) {
                    if let Err(e) = self.publish_once(p) {
                        errors.push(e);
                    }
                }
            }
            PickKind::Folder => {
                for dir in paths {
                    let Ok(entries) = std::fs::read_dir(dir) else { continue };
                    for entry in entries.flatten().take(MAX_FOLDER_FILES) {
                        let p = entry.path();
                        if p.is_file()
                            && let Err(e) = self.publish_once(&p)
                        {
                            errors.push(e);
                        }
                    }
                }
            }
            PickKind::File | PickKind::Files | PickKind::FolderToRead => {}
        }
        errors
    }

    fn clipboard_text(&self) -> Option<String> {
        bridge::clipboard_text(&self.app).unwrap_or_else(|e| {
            log::warn!("couldn't read the clipboard: {e}");
            None
        })
    }

    fn clipboard_image(&self) -> Option<(usize, usize, Vec<u8>)> {
        None
    }

    fn set_clipboard_image(&self, _width: usize, _height: usize, _rgba: &[u8]) -> Result<(), String> {
        Err("copying images to the clipboard isn't available on Android yet".to_string())
    }

    fn set_clipboard_files(&self, _paths: &[PathBuf]) -> Result<(), String> {
        Err("copying files to the clipboard isn't available on Android yet".to_string())
    }
}
