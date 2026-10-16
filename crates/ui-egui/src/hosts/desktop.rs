//! The desktop host: rfd's native panels and arboard's clipboard, as the shell used them directly
//! before [`PlatformHost`] existed. Behaviour is unchanged.

use std::path::PathBuf;

use crate::platform_host::{Dialog, PickFuture, PickKind, PlatformHost};

/// macOS, Windows, Linux and the BSDs.
#[derive(Clone, Copy, Debug, Default)]
pub struct DesktopHost;

/// rfd's builder for `dialog`.
fn rfd_dialog(dialog: Dialog) -> rfd::AsyncFileDialog {
    let mut d = rfd::AsyncFileDialog::new();
    for (name, extensions) in &dialog.filters {
        d = d.add_filter(name, extensions.as_slice());
    }
    if let Some(title) = dialog.title {
        d = d.set_title(title);
    }
    if let Some(name) = dialog.file_name {
        d = d.set_file_name(name);
    }
    d
}

fn path_of(handle: Option<rfd::FileHandle>) -> Vec<PathBuf> {
    handle.map(|h| h.path().to_path_buf()).into_iter().collect()
}

impl PlatformHost for DesktopHost {
    fn pick(&self, kind: PickKind, dialog: Dialog) -> PickFuture {
        // On macOS creating the future shows the panel: this runs on the main thread (`ask`).
        let d = rfd_dialog(dialog);
        match kind {
            PickKind::File => {
                let pick = d.pick_file();
                Box::pin(async move { path_of(pick.await) })
            }
            PickKind::Files => {
                let pick = d.pick_files();
                Box::pin(async move { pick.await.unwrap_or_default().into_iter().map(|h| h.path().to_path_buf()).collect() })
            }
            PickKind::Folder | PickKind::FolderToRead => {
                let pick = d.pick_folder();
                Box::pin(async move { path_of(pick.await) })
            }
            PickKind::Save => {
                let pick = d.save_file();
                Box::pin(async move { path_of(pick.await) })
            }
        }
    }

    fn clipboard_text(&self) -> Option<String> {
        arboard::Clipboard::new().ok()?.get_text().ok()
    }

    fn clipboard_image(&self) -> Option<(usize, usize, Vec<u8>)> {
        let img = arboard::Clipboard::new().ok()?.get_image().ok()?;
        Some((img.width, img.height, img.bytes.into_owned()))
    }

    fn set_clipboard_image(&self, width: usize, height: usize, rgba: &[u8]) -> Result<(), String> {
        let img = arboard::ImageData { width, height, bytes: std::borrow::Cow::Borrowed(rgba) };
        arboard::Clipboard::new().and_then(|mut c| c.set_image(img)).map_err(|e| e.to_string())
    }

    fn set_clipboard_files(&self, paths: &[PathBuf]) -> Result<(), String> {
        arboard::Clipboard::new().and_then(|mut c| c.set().file_list(paths)).map_err(|e| e.to_string())
    }
}
