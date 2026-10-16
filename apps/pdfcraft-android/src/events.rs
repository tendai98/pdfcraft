//! The text protocol between `PdfCraftActivity` and the host, kept free of JNI so it is tested
//! on every platform. Events are single lines, fields separated by tabs:
//!
//! - `picked\t<id>\t<paths>`: picker `id` closed; `<paths>` are the staged files (or the folder
//!   to write into) separated by U+001F, empty when cancelled.
//! - `open\t<path>`: a document opened from another app (ACTION_VIEW / ACTION_SEND), staged.
//! - `theme\t<0|1>`: the system switched to light (0) or dark (1).
//! - `trim\t<level>`: the system asked for memory back (`onTrimMemory`).
//! - `error\t<message>`: something to tell the user.
//!
//! Everything here comes from another process's file names and is untrusted: malformed lines are
//! dropped, and paths must be absolute.

use std::path::PathBuf;

/// More paths than this in one pick are not taken.
const MAX_PATHS: usize = 1000;
/// Longer messages are cut.
const MAX_MESSAGE_CHARS: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Picked { id: u64, paths: Vec<PathBuf> },
    Open(PathBuf),
    Theme { dark: bool },
    Trim { level: i32 },
    Notice(String),
}

/// One event line, or `None` when it is malformed.
pub fn parse(line: &str) -> Option<Event> {
    let (kind, rest) = line.split_once('\t')?;
    match kind {
        "picked" => {
            let (id, paths) = rest.split_once('\t').unwrap_or((rest, ""));
            let id = id.parse().ok()?;
            let paths = paths.split('\u{1F}').filter(|p| !p.is_empty()).filter_map(path).take(MAX_PATHS).collect();
            Some(Event::Picked { id, paths })
        }
        "open" => path(rest).map(Event::Open),
        "theme" => match rest {
            "0" => Some(Event::Theme { dark: false }),
            "1" => Some(Event::Theme { dark: true }),
            _ => None,
        },
        "trim" => rest.parse().ok().map(|level| Event::Trim { level }),
        "error" => Some(Event::Notice(rest.chars().take(MAX_MESSAGE_CHARS).collect())),
        _ => None,
    }
}

/// An absolute path without NULs, or `None`.
fn path(s: &str) -> Option<PathBuf> {
    (s.starts_with('/') && !s.contains('\0')).then(|| PathBuf::from(s))
}

/// The MIME types a picker offers for these extensions, comma-separated; any file when an
/// extension has no well-known type (the shell still checks what it reads).
pub fn mime_types(extensions: &[String]) -> String {
    let mut types: Vec<&str> = Vec::new();
    for ext in extensions {
        let Some(t) = mime_of_extension(ext) else { return "*/*".to_string() };
        if !types.contains(&t) {
            types.push(t);
        }
    }
    if types.is_empty() { "*/*".to_string() } else { types.join(",") }
}

/// The MIME type to create a file named `name` with.
pub fn mime_for_name(name: &str) -> &'static str {
    name.rsplit_once('.').and_then(|(_, ext)| mime_of_extension(&ext.to_ascii_lowercase())).unwrap_or("application/octet-stream")
}

fn mime_of_extension(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "jp2" | "j2k" | "jpx" => "image/jp2",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "json" => "application/json",
        "rtf" => "application/rtf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picked_with_paths_cancelled_and_malformed() {
        assert_eq!(
            parse("picked\t7\t/c/saf/1/a.pdf\u{1F}/c/saf/2/b.pdf"),
            Some(Event::Picked { id: 7, paths: vec![PathBuf::from("/c/saf/1/a.pdf"), PathBuf::from("/c/saf/2/b.pdf")] })
        );
        assert_eq!(parse("picked\t7\t"), Some(Event::Picked { id: 7, paths: vec![] }));
        assert_eq!(parse("picked\t7"), Some(Event::Picked { id: 7, paths: vec![] }));
        assert_eq!(parse("picked\tx\t/a"), None);
        assert_eq!(parse("picked\t-1\t/a"), None);
        // Relative paths and NULs are dropped, the rest kept.
        assert_eq!(parse("picked\t1\trel.pdf\u{1F}/a\0b\u{1F}/ok.pdf"), Some(Event::Picked { id: 1, paths: vec![PathBuf::from("/ok.pdf")] }));
    }

    #[test]
    fn a_huge_pick_is_capped() {
        let many = vec!["/p.pdf"; MAX_PATHS + 5].join("\u{1F}");
        let Some(Event::Picked { paths, .. }) = parse(&format!("picked\t1\t{many}")) else { panic!("not parsed") };
        assert_eq!(paths.len(), MAX_PATHS);
    }

    #[test]
    fn other_events() {
        assert_eq!(parse("open\t/c/saf/3/x.pdf"), Some(Event::Open(PathBuf::from("/c/saf/3/x.pdf"))));
        assert_eq!(parse("open\tx.pdf"), None);
        assert_eq!(parse("theme\t1"), Some(Event::Theme { dark: true }));
        assert_eq!(parse("theme\t0"), Some(Event::Theme { dark: false }));
        assert_eq!(parse("theme\t2"), None);
        assert_eq!(parse("trim\t80"), Some(Event::Trim { level: 80 }));
        assert_eq!(parse("trim\tlots"), None);
        assert_eq!(parse("error\tNo app"), Some(Event::Notice("No app".into())));
        assert_eq!(parse("error\t").map(|_| ()), Some(()));
        assert_eq!(parse(&format!("error\t{}", "é".repeat(MAX_MESSAGE_CHARS * 2))), Some(Event::Notice("é".repeat(MAX_MESSAGE_CHARS))));
        assert_eq!(parse("bogus\tx"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("picked"), None);
    }

    #[test]
    fn mime_types_for_pickers() {
        let ext = |e: &[&str]| e.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(mime_types(&ext(&["pdf"])), "application/pdf");
        assert_eq!(mime_types(&ext(&["png", "jpg", "jpeg"])), "image/png,image/jpeg");
        assert_eq!(mime_types(&ext(&["pdf", "xfdf"])), "*/*", "an unknown type offers every file");
        assert_eq!(mime_types(&[]), "*/*");
        assert_eq!(mime_for_name("a.PDF"), "application/pdf");
        assert_eq!(mime_for_name("measurements.csv"), "text/csv");
        assert_eq!(mime_for_name("noext"), "application/octet-stream");
        assert_eq!(mime_for_name("a.xfdf"), "application/octet-stream");
    }
}
