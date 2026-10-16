//! What the shell asks of the operating system it runs on: file pickers, the clipboard, and
//! publishing a written file. [`PdfCraftApp::host`](crate::PdfCraftApp::host) holds one.
//!
//! The desktop implementation, [`DesktopHost`](crate::hosts::desktop::DesktopHost), wraps rfd and
//! arboard, which is what the shell called directly before. The Android app implements
//! [`PlatformHost`] over the Storage Access Framework (`apps/pdfcraft-android`), where pickers
//! answer with copies of the chosen documents in the app's cache and every write to one of those
//! copies is published back to its document ([`PlatformHost::after_write`]). The browser build has
//! its own byte-based flows (`FileRequest`, `download`) and doesn't use this.
//!
//! [`Dialog`] describes a picker the way rfd's builder did (filters, title, proposed name) but is
//! plain data, so every target builds it and each host turns it into its own panel.

#[cfg(not(target_arch = "wasm32"))]
use std::future::Future;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::pin::Pin;

/// A file picker to show: what it is called, which files it offers, and the proposed name when
/// saving. Built like rfd's `AsyncFileDialog`, which it replaces at every call site.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dialog {
    /// The window title, where the system shows one.
    pub title: Option<String>,
    /// Named groups of extensions (without the dot), offered in order.
    pub filters: Vec<(String, Vec<String>)>,
    /// The proposed file name when saving.
    pub file_name: Option<String>,
}

impl Dialog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer files with these extensions (without the dot) under `name`.
    #[must_use]
    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.filters.push((name.into(), extensions.iter().map(ToString::to_string).collect()));
        self
    }

    #[must_use]
    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    #[must_use]
    pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    /// rfd's browser picker for this dialog (the web build picks through rfd directly).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn to_rfd(self) -> rfd::AsyncFileDialog {
        let mut d = rfd::AsyncFileDialog::new();
        for (name, extensions) in &self.filters {
            d = d.add_filter(name, extensions.as_slice());
        }
        if let Some(title) = self.title {
            d = d.set_title(title);
        }
        if let Some(name) = self.file_name {
            d = d.set_file_name(name);
        }
        d
    }

    /// Every extension the filters offer, lower-case, each once, in order.
    pub fn extensions(&self) -> Vec<String> {
        let mut all: Vec<String> = Vec::new();
        for ext in self.filters.iter().flat_map(|(_, exts)| exts) {
            let ext = ext.trim_start_matches('.').to_ascii_lowercase();
            if !ext.is_empty() && !all.contains(&ext) {
                all.push(ext);
            }
        }
        all
    }
}

/// Which kind of picker to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickKind {
    /// One existing file.
    File,
    /// One or more existing files.
    Files,
    /// A folder whose files are read (Combine ▸ Add folder, a folder pinned to Home).
    FolderToRead,
    /// A folder that files are written into (Split, Extract to separate files, batch results).
    Folder,
    /// Where to save one file.
    Save,
}

/// The chosen paths, once the picker closes; empty when it was cancelled.
#[cfg(not(target_arch = "wasm32"))]
pub type PickFuture = Pin<Box<dyn Future<Output = Vec<PathBuf>> + Send>>;

/// The operating-system services the shell uses. Every method is called on the UI thread.
#[cfg(not(target_arch = "wasm32"))]
pub trait PlatformHost: Send + Sync {
    /// Show `dialog` as a picker of `kind`. The panel shows now (on macOS it must be created on
    /// the main thread); the future, awaited on a worker thread, yields the chosen paths.
    fn pick(&self, kind: PickKind, dialog: Dialog) -> PickFuture;

    /// The shell has just written `path` (a validated save). A host whose picked paths stand for
    /// documents elsewhere copies it there; an error means the document itself was not updated.
    fn after_write(&self, _path: &Path) -> Result<(), String> {
        Ok(())
    }

    /// A pick's follow-up has run with `paths`, as chosen from a picker of `kind`; it may have
    /// written files there. Returns one message per file that couldn't be published.
    fn after_pick(&self, _kind: PickKind, _paths: &[PathBuf]) -> Vec<String> {
        Vec::new()
    }

    /// The text on the system clipboard.
    fn clipboard_text(&self) -> Option<String>;

    /// The image on the system clipboard: width, height and RGBA pixels.
    fn clipboard_image(&self) -> Option<(usize, usize, Vec<u8>)>;

    /// Put an RGBA image on the system clipboard.
    fn set_clipboard_image(&self, width: usize, height: usize, rgba: &[u8]) -> Result<(), String>;

    /// Put files on the system clipboard, for pasting into a file manager or another app.
    fn set_clipboard_files(&self, paths: &[PathBuf]) -> Result<(), String>;
}

/// A host with no system services: pickers answer at once with nothing chosen, and the
/// clipboard is empty. The default where no other host exists (Android before its app sets
/// one), and for tests that need a host that never shows a panel.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct NoHost;

#[cfg(not(target_arch = "wasm32"))]
impl PlatformHost for NoHost {
    fn pick(&self, _kind: PickKind, _dialog: Dialog) -> PickFuture {
        Box::pin(async { Vec::new() })
    }

    fn clipboard_text(&self) -> Option<String> {
        None
    }

    fn clipboard_image(&self) -> Option<(usize, usize, Vec<u8>)> {
        None
    }

    fn set_clipboard_image(&self, _width: usize, _height: usize, _rgba: &[u8]) -> Result<(), String> {
        Err(NOT_AVAILABLE.to_string())
    }

    fn set_clipboard_files(&self, _paths: &[PathBuf]) -> Result<(), String> {
        Err(NOT_AVAILABLE.to_string())
    }
}

/// What a host without a system clipboard says.
#[cfg(not(target_arch = "wasm32"))]
const NOT_AVAILABLE: &str = "the system clipboard isn't available here";

/// The host this build uses unless its app sets another: the desktop one, or [`NoHost`] where
/// there is no desktop (Android sets its own).
#[cfg(not(target_arch = "wasm32"))]
pub fn default_host() -> std::sync::Arc<dyn PlatformHost> {
    #[cfg(not(target_os = "android"))]
    return std::sync::Arc::new(crate::hosts::desktop::DesktopHost);
    #[cfg(target_os = "android")]
    return std::sync::Arc::new(NoHost);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_lower_case_without_dots_and_each_once() {
        let d = Dialog::new().add_filter("PDF", &["pdf"]).add_filter("Images", &["PNG", ".jpg", "pdf", ""]);
        assert_eq!(d.extensions(), vec!["pdf", "png", "jpg"]);
        assert!(Dialog::new().extensions().is_empty());
    }

    #[test]
    fn builder_keeps_title_name_and_filter_order() {
        let d = Dialog::new().set_title("Save").set_file_name("a.pdf").add_filter("B", &["b"]).add_filter("A", &["a"]);
        assert_eq!(d.title.as_deref(), Some("Save"));
        assert_eq!(d.file_name.as_deref(), Some("a.pdf"));
        assert_eq!(d.filters.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["B", "A"]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn no_host_picks_nothing_and_refuses_clipboard_writes() {
        let host = NoHost;
        assert!(pollster::block_on(host.pick(PickKind::Files, Dialog::new())).is_empty());
        assert!(host.clipboard_text().is_none() && host.clipboard_image().is_none());
        assert!(host.set_clipboard_image(1, 1, &[0, 0, 0, 0]).is_err());
        assert!(host.after_write(Path::new("x.pdf")).is_ok());
        assert!(host.after_pick(PickKind::Save, &[]).is_empty());
    }
}
