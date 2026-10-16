//! Native file pickers that never block the frame.
//!
//! A blocking `rfd::FileDialog` runs `-[NSOpenPanel runModal]` on macOS: a nested run loop inside
//! winit's event handler. An event that arrives while it spins re-enters the handler, winit
//! panics, and because the panic can't unwind out of the AppKit callback the app aborts.
//! `rfd::AsyncFileDialog` returns at once instead, and the app uses the choice on a later frame
//! ([`PdfCraftApp::process_picked`]); a worker thread waits for it.
//!
//! How rfd 0.17 shows the panel (checked against its source):
//! - macOS: creating the future shows the panel, so it must be created on the main thread. With
//!   no parent given, rfd uses the app's main (or first) window and opens the panel as a sheet
//!   on it (`beginSheetModalForWindow`). Without a running app and a window it falls back to the
//!   blocking `runModal`, which is why this only runs inside the running eframe app.
//! - Windows: the panel runs on a thread rfd starts when the future is first polled (here, by
//!   the worker), not inside winit's callback.
//! - Linux and the BSDs: the XDG portal backend (with a Zenity fallback) also works off the UI
//!   thread; the optional GTK3 backend shows the panel on rfd's own GTK thread.
//!
//! Every native picker goes through [`PdfCraftApp::ask`]: open, save, folder and multi-file
//! panels alike. What happens with the choice is a closure that runs on that later frame, so
//! work that depends on the file being written (closing the tab after Save, deleting extracted
//! pages) happens there and nowhere else. One picker shows at a time, and it keeps that claim
//! until its closure starts, so a closure that asks again (merge data, then save the
//! spreadsheet) always gets its second picker.

use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use pdfcraft_engine::DocId;

use crate::PdfCraftApp;
use crate::files::{FilePurpose, PickTarget};
use crate::platform_host::{Dialog, PickKind};

/// What the chosen files are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickFor {
    /// File ▸ Open: open the chosen file in a new tab.
    Open,
    /// Combine, insert, replace or OCR (see [`FilePurpose`]).
    Files(FilePurpose),
}

/// Which native panel to show. The host ([`PdfCraftApp::host`]) turns the [`Dialog`] into one.
pub(crate) enum Ask {
    /// Choose one existing file.
    File(Dialog),
    /// Choose one or more existing files.
    Files(Dialog),
    /// Choose a folder whose files are read.
    FolderToRead(Dialog),
    /// Choose a folder to write files into.
    Folder(Dialog),
    /// Choose where to save a file.
    Save(Dialog),
}

impl Ask {
    fn split(self) -> (PickKind, Dialog) {
        match self {
            Ask::File(d) => (PickKind::File, d),
            Ask::Files(d) => (PickKind::Files, d),
            Ask::FolderToRead(d) => (PickKind::FolderToRead, d),
            Ask::Folder(d) => (PickKind::Folder, d),
            Ask::Save(d) => (PickKind::Save, d),
        }
    }
}

/// What to do with the chosen paths, on a later frame.
type Then = Box<dyn FnOnce(&mut PdfCraftApp, Vec<PathBuf>) + Send>;

/// A finished pick, waiting for the next frame.
struct Picked {
    target: Option<PickTarget>,
    /// The picker it came from: the host may publish what the follow-up wrote (`after_pick`).
    kind: PickKind,
    /// The chosen paths; empty when the picker was cancelled.
    paths: Vec<PathBuf>,
    then: Then,
}

impl std::fmt::Debug for Picked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Picked").field("target", &self.target).field("kind", &self.kind).field("paths", &self.paths).finish_non_exhaustive()
    }
}

/// Pickers in flight and the picks they finished.
#[derive(Clone, Debug, Default)]
pub struct Pickers {
    /// A picker is showing, or its pick is waiting for the frame; a second request is refused
    /// until the pick's closure starts.
    showing: Arc<AtomicBool>,
    done: Arc<Mutex<Vec<Picked>>>,
}

/// Clears `showing` if the worker ends without queueing a pick (it panicked).
struct Showing(Option<Arc<AtomicBool>>);

impl Showing {
    /// The pick is queued: the frame that uses it clears the flag.
    fn hand_over(mut self) {
        self.0 = None;
    }
}

impl Drop for Showing {
    fn drop(&mut self) {
        if let Some(flag) = &self.0 {
            flag.store(false, Ordering::SeqCst);
        }
    }
}

impl Pickers {
    /// Wait for `pick` on a worker thread and queue its result for the next frame. On macOS
    /// `pick` must already have shown the picker (rfd starts the panel when the future is
    /// created, on the main thread). Returns false when the worker couldn't start.
    fn spawn<F>(&self, target: Option<PickTarget>, kind: PickKind, ctx: Option<egui::Context>, pick: F, then: Then) -> bool
    where
        F: Future<Output = Vec<PathBuf>> + Send + 'static,
    {
        self.spawn_worker(target, kind, ctx, pick, then).is_some()
    }

    /// [`Self::spawn`], returning the worker so tests can wait for it instead of polling.
    fn spawn_worker<F>(
        &self,
        target: Option<PickTarget>,
        kind: PickKind,
        ctx: Option<egui::Context>,
        pick: F,
        then: Then,
    ) -> Option<std::thread::JoinHandle<()>>
    where
        F: Future<Output = Vec<PathBuf>> + Send + 'static,
    {
        let showing = Showing(Some(self.showing.clone()));
        let done = self.done.clone();
        std::thread::Builder::new()
            .name("file-picker".into())
            .spawn(move || {
                let paths = pollster::block_on(pick);
                done.lock().unwrap_or_else(PoisonError::into_inner).push(Picked { target, kind, paths, then });
                showing.hand_over();
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            })
            .ok()
    }

    /// Queue a pick that is already known (tests and automation). Like a real pick, it keeps
    /// the flag until the frame uses it.
    fn deliver(&self, target: Option<PickTarget>, kind: PickKind, paths: Vec<PathBuf>, then: Then) {
        self.done.lock().unwrap_or_else(PoisonError::into_inner).push(Picked { target, kind, paths, then });
    }

    fn take(&self) -> Vec<Picked> {
        std::mem::take(&mut *self.done.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl PdfCraftApp {
    /// Show a native picker for `pick_for` without blocking the frame. The choice is used on a
    /// later frame by [`Self::process_picked`].
    pub(crate) fn pick(&mut self, pick_for: PickFor, dialog: Dialog, multiple: bool) {
        // Insert and Replace edit the active document: remember which one.
        let target = match pick_for {
            PickFor::Files(purpose) if purpose.edits_active_document() => self.active_ids().map(|(_, id)| id),
            _ => None,
        };
        let ask = if multiple { Ask::Files(dialog) } else { Ask::File(dialog) };
        self.ask(ask, target, move |app, paths| match pick_for {
            PickFor::Open => paths.iter().for_each(|p| app.open_path(&p.to_string_lossy())),
            PickFor::Files(purpose) => app.use_paths(purpose, &paths),
        });
    }

    /// Show `ask` without blocking the frame, and call `then` with the chosen paths on a later
    /// frame ([`Self::process_picked`]). Returns false, after telling the user, when no picker
    /// could be shown: another one is still open, or its worker couldn't start.
    ///
    /// `then` is not called when the picker is cancelled, or when `target` is given and by then
    /// that document is no longer the active one or has been edited (the user is told). Use
    /// `target` whenever `then` edits the active document or refers to its pages or objects by
    /// position.
    ///
    /// `pick_override` answers instead of a native panel (tests and automation); the answer
    /// still arrives on a later frame, like a real pick.
    pub(crate) fn ask(&mut self, ask: Ask, target: Option<DocId>, then: impl FnOnce(&mut PdfCraftApp, Vec<PathBuf>) + Send + 'static) -> bool {
        if self.pickers.showing.swap(true, Ordering::SeqCst) {
            self.notify_tr("Another file dialog is still open. Finish with it first.");
            return false;
        }
        let target = self.pick_target(target);
        let then: Then = Box::new(then);
        let (kind, dialog) = ask.split();
        if let Some(paths) = self.pick_override.clone() {
            self.pickers.deliver(target, kind, paths.into_iter().map(PathBuf::from).collect(), then);
            return true;
        }
        // On macOS creating the future shows the panel; it must happen here, on the main thread.
        let pick = self.host.pick(kind, dialog);
        let started = self.pickers.spawn(target, kind, self.ctx.clone(), pick, then);
        if !started {
            self.pickers.showing.store(false, Ordering::SeqCst);
            self.notify_tr("Couldn't show the file picker. Please try again.");
        }
        started
    }

    /// [`Self::ask`] for one path: `then` gets the first chosen path.
    pub(crate) fn ask_one(&mut self, ask: Ask, target: Option<DocId>, then: impl FnOnce(&mut PdfCraftApp, PathBuf) + Send + 'static) -> bool {
        self.ask(ask, target, move |app, paths| {
            if let Some(path) = paths.into_iter().next() {
                then(app, path);
            }
        })
    }

    /// Use the picks that finished since the last frame. Called every frame.
    pub(crate) fn process_picked(&mut self) {
        for Picked { target, kind, paths, then } in self.pickers.take() {
            // The pick is in hand: the next picker may show, including one `then` asks for.
            self.pickers.showing.store(false, Ordering::SeqCst);
            if paths.is_empty() {
                continue;
            }
            if !self.still_pick_target(target) {
                continue;
            }
            // Last-resort guard (AGENTS.md §4), as for commands: this work used to run inside
            // the command that asked, and a panic in it must not take the app down.
            let chosen = paths.clone();
            if let Err(m) = pdfcraft_engine::guard(|| then(self, paths)) {
                self.notify_fmt("That didn't work: an internal error stopped it ({m}).", &[("m", m.as_str())]);
            }
            // What the follow-up wrote reaches the chosen documents (Android; a no-op elsewhere).
            for e in self.host.after_pick(kind, &chosen) {
                self.notify_fmt("Couldn't save {name}: {e}", &[("name", &chosen_name(&chosen)), ("e", &e)]);
            }
        }
    }
}

/// How a pick is named in a message: the first chosen file's name.
fn chosen_name(paths: &[PathBuf]) -> String {
    paths.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(into: &Arc<Mutex<Vec<PathBuf>>>) -> Then {
        let into = into.clone();
        Box::new(move |_, paths| into.lock().unwrap().extend(paths))
    }

    #[test]
    fn worker_queues_the_pick_and_keeps_the_flag_until_the_frame_uses_it() {
        let mut app = PdfCraftApp::new();
        app.pickers.showing.store(true, Ordering::SeqCst);
        let got = Arc::new(Mutex::new(Vec::new()));
        let worker =
            app.pickers.spawn_worker(None, PickKind::File, None, async { vec![PathBuf::from("a.pdf")] }, record(&got)).expect("worker starts");
        // Wait for the worker itself: polling with a deadline was flaky on loaded CI runners.
        assert!(worker.join().is_ok());
        assert!(app.pickers.showing.load(Ordering::SeqCst), "the queued pick still owns the flag");
        app.process_picked();
        assert!(!app.pickers.showing.load(Ordering::SeqCst));
        assert_eq!(*got.lock().unwrap(), vec![PathBuf::from("a.pdf")]);
    }

    #[test]
    fn a_panicking_worker_clears_showing_and_queues_nothing() {
        let pickers = Pickers::default();
        pickers.showing.store(true, Ordering::SeqCst);
        fn fail() -> Vec<PathBuf> {
            panic!("picker failed")
        }
        let got = Arc::new(Mutex::new(Vec::new()));
        let worker = pickers.spawn_worker(None, PickKind::File, None, async { fail() }, record(&got)).expect("worker starts");
        // join returns once the panic has unwound and the drop guard has run.
        assert!(worker.join().is_err());
        assert!(!pickers.showing.load(Ordering::SeqCst));
        assert!(pickers.take().is_empty());
    }

    #[test]
    fn the_follow_up_runs_on_a_later_frame_and_not_when_cancelled() {
        let mut app = PdfCraftApp::new();
        let got = Arc::new(Mutex::new(Vec::new()));
        app.pickers.deliver(None, PickKind::File, vec![], record(&got));
        app.pickers.deliver(None, PickKind::File, vec![PathBuf::from("b.pdf")], record(&got));
        assert!(got.lock().unwrap().is_empty(), "nothing runs until the frame uses the picks");
        app.process_picked();
        assert_eq!(*got.lock().unwrap(), vec![PathBuf::from("b.pdf")], "a cancelled pick runs nothing");
    }

    #[test]
    fn a_second_picker_is_refused_while_one_is_showing() {
        let mut app = PdfCraftApp::new();
        app.pick_override = Some(vec!["c.pdf".into()]);
        app.pickers.showing.store(true, Ordering::SeqCst);
        let got = Arc::new(Mutex::new(Vec::new()));
        let into = got.clone();
        assert!(!app.ask(Ask::File(Dialog::new()), None, move |_, paths| into.lock().unwrap().extend(paths)));
        app.process_picked();
        assert!(got.lock().unwrap().is_empty());
        assert!(app.pickers.showing.load(Ordering::SeqCst), "the showing picker still owns the flag");
        assert!(app.toast.as_ref().is_some_and(|(m, _)| m.contains("Another file dialog")), "the user is told");
    }

    #[test]
    fn a_follow_up_that_asks_again_gets_its_picker_even_if_another_request_came_first() {
        let mut app = PdfCraftApp::new();
        app.pick_override = Some(vec!["first.fdf".into()]);
        let got = Arc::new(Mutex::new(Vec::new()));
        let into = got.clone();
        assert!(app.ask(Ask::Files(Dialog::new()), None, move |app, _| {
            // Merge data: the spreadsheet's save picker, asked from the first pick's closure.
            assert!(app.ask(Ask::Save(Dialog::new()), None, move |_, paths| into.lock().unwrap().extend(paths)));
        }));
        // Another command asks for a picker before the frame uses the first pick: refused.
        assert!(!app.ask(Ask::File(Dialog::new()), None, |_, _| panic!("must not run")));
        app.process_picked();
        app.process_picked();
        assert_eq!(*got.lock().unwrap(), vec![PathBuf::from("first.fdf")], "the chained picker answered");
    }

    #[test]
    fn a_panicking_follow_up_is_reported_and_the_next_pick_still_runs() {
        let mut app = PdfCraftApp::new();
        let got = Arc::new(Mutex::new(Vec::new()));
        app.pickers.deliver(None, PickKind::File, vec![PathBuf::from("x.pdf")], Box::new(|_, _| panic!("follow-up failed")));
        app.pickers.deliver(None, PickKind::File, vec![PathBuf::from("y.pdf")], record(&got));
        app.process_picked();
        assert_eq!(*got.lock().unwrap(), vec![PathBuf::from("y.pdf")]);
        let toast = app.toast.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(toast.contains("internal error"), "the user is told: {toast:?}");
        assert!(!app.pickers.showing.load(Ordering::SeqCst));
    }

    /// Records what the shell asks of it after a pick, and fails publishing when told to.
    #[derive(Default)]
    struct Recording {
        after_pick: Mutex<Vec<(PickKind, Vec<PathBuf>)>>,
        fail: bool,
    }

    impl crate::platform_host::PlatformHost for Recording {
        fn pick(&self, _kind: PickKind, _dialog: Dialog) -> crate::platform_host::PickFuture {
            Box::pin(async { Vec::new() })
        }
        fn after_pick(&self, kind: PickKind, paths: &[PathBuf]) -> Vec<String> {
            self.after_pick.lock().unwrap().push((kind, paths.to_vec()));
            if self.fail { vec!["the folder refused it".into()] } else { Vec::new() }
        }
        fn clipboard_text(&self) -> Option<String> {
            None
        }
        fn clipboard_image(&self) -> Option<(usize, usize, Vec<u8>)> {
            None
        }
        fn set_clipboard_image(&self, _: usize, _: usize, _: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn set_clipboard_files(&self, _: &[PathBuf]) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn the_host_publishes_after_the_follow_up_with_the_picks_kind_and_paths() {
        let mut app = PdfCraftApp::new();
        let host = Arc::new(Recording::default());
        app.host = host.clone();
        app.pick_override = Some(vec!["/out/a.pdf".into()]);
        let ran = Arc::new(AtomicBool::new(false));
        let r = ran.clone();
        assert!(app.ask(Ask::Save(Dialog::new()), None, move |_, _| {
            r.store(true, Ordering::SeqCst);
        }));
        assert!(host.after_pick.lock().unwrap().is_empty(), "nothing is published before the frame uses the pick");
        app.process_picked();
        assert!(ran.load(Ordering::SeqCst));
        assert_eq!(*host.after_pick.lock().unwrap(), vec![(PickKind::Save, vec![PathBuf::from("/out/a.pdf")])]);
    }

    #[test]
    fn a_publish_failure_after_a_pick_is_reported() {
        let mut app = PdfCraftApp::new();
        app.host = Arc::new(Recording { fail: true, ..Default::default() });
        app.pick_override = Some(vec!["/out/parts".into()]);
        assert!(app.ask(Ask::Folder(Dialog::new()), None, |_, _| {}));
        app.process_picked();
        let toast = app.toast.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(toast.contains("refused"), "the user is told: {toast:?}");
    }
}
