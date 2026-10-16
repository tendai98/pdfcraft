//! Calls into `PdfCraftActivity` (android/app/src/main/java/app/pdfcraft/android/), one function
//! per Kotlin method. Every call is safe JNI ([`crate::ffi::with_activity`] does the one borrow),
//! and every failure is an error string, never a panic.

use android_activity::AndroidApp;
use jni::objects::{JObject, JString};
use jni::{Env, jni_sig, jni_str};

use crate::ffi::with_activity;

/// A Java string result as Rust, `None` for `null`.
fn string_of(env: &mut Env, obj: JObject) -> jni::errors::Result<Option<String>> {
    if obj.is_null() {
        return Ok(None);
    }
    let s = env.cast_local::<JString>(obj)?;
    s.try_to_string(env).map(Some)
}

/// The activity's next queued event (see host.rs for the format), or `None`.
pub(crate) fn poll_event(app: &AndroidApp) -> Result<Option<String>, String> {
    with_activity(app, |env, activity| {
        let v = env.call_method(activity, jni_str!("pollEvent"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
        string_of(env, v)
    })
}

/// Choose documents to open: `mime_types` comma-separated, answered by a `picked` event.
pub(crate) fn request_open(app: &AndroidApp, id: i64, mime_types: &str, multiple: bool) -> Result<(), String> {
    with_activity(app, |env, activity| {
        let mimes = env.new_string(mime_types)?;
        env.call_method(activity, jni_str!("requestOpen"), jni_sig!("(JLjava/lang/String;Z)V"), &[id.into(), (&mimes).into(), multiple.into()])?;
        Ok(())
    })
}

/// Choose where to save `name`, answered by a `picked` event with the path to write.
pub(crate) fn request_save(app: &AndroidApp, id: i64, name: &str, mime: &str) -> Result<(), String> {
    with_activity(app, |env, activity| {
        let name = env.new_string(name)?;
        let mime = env.new_string(mime)?;
        env.call_method(
            activity,
            jni_str!("requestSave"),
            jni_sig!("(JLjava/lang/String;Ljava/lang/String;)V"),
            &[id.into(), (&name).into(), (&mime).into()],
        )?;
        Ok(())
    })
}

/// Choose a folder to write into, answered by a `picked` event with the folder to write in.
pub(crate) fn request_folder(app: &AndroidApp, id: i64) -> Result<(), String> {
    with_activity(app, |env, activity| {
        env.call_method(activity, jni_str!("requestFolder"), jni_sig!("(J)V"), &[id.into()])?;
        Ok(())
    })
}

/// What [`publish`] did with a file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Published {
    /// Copied to its document.
    Copied,
    /// Not a staged document: nothing to do.
    NotStaged,
}

/// Copy the staged file at `path` to the document it stands for.
pub(crate) fn publish(app: &AndroidApp, path: &str) -> Result<Published, String> {
    let answer = with_activity(app, |env, activity| {
        let path = env.new_string(path)?;
        let v = env.call_method(activity, jni_str!("publish"), jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"), &[(&path).into()])?.l()?;
        string_of(env, v)
    })?;
    match answer.as_deref() {
        Some("ok") => Ok(Published::Copied),
        Some("skip") => Ok(Published::NotStaged),
        Some(other) => Err(other.strip_prefix("error\t").unwrap_or(other).to_string()),
        None => Err("the document couldn't be written".to_string()),
    }
}

pub(crate) fn clipboard_text(app: &AndroidApp) -> Result<Option<String>, String> {
    with_activity(app, |env, activity| {
        let v = env.call_method(activity, jni_str!("clipboardText"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
        string_of(env, v)
    })
}

pub(crate) fn set_clipboard_text(app: &AndroidApp, text: &str) -> Result<(), String> {
    with_activity(app, |env, activity| {
        let text = env.new_string(text)?;
        env.call_method(activity, jni_str!("setClipboardText"), jni_sig!("(Ljava/lang/String;)V"), &[(&text).into()])?;
        Ok(())
    })
}

pub(crate) fn is_dark_mode(app: &AndroidApp) -> Result<bool, String> {
    with_activity(app, |env, activity| env.call_method(activity, jni_str!("isDarkMode"), jni_sig!("()Z"), &[])?.z())
}
