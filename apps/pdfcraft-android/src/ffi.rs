//! The only `unsafe` in PdfCraft (approved by the owner, 2026-10-16; docs/android/README.md):
//!
//! 1. exporting `android_main`, the symbol android-activity's native glue calls by name once the
//!    activity starts (`#[unsafe(no_mangle)]`), and
//! 2. borrowing the activity's JNI global reference, which android-activity only hands out as a
//!    raw pointer.
//!
//! Everything else talks to Java through the jni crate's safe API, and only by calling methods
//! of `PdfCraftActivity`: Java never calls into Rust, so no `Java_*` function is exported.

#![allow(unsafe_code)]

use android_activity::AndroidApp;
use jni::objects::JObject;
use jni::refs::Global;
use jni::{Env, JavaVM};

/// Called by android-activity on its own thread once the activity is created; returning ends
/// the activity's native side.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    crate::run(app);
}

/// Run `f` with a JNI environment for this thread and the activity object. Java exceptions are
/// cleared and come back as errors, so a failing call never leaves one pending.
pub(crate) fn with_activity<T>(app: &AndroidApp, f: impl FnOnce(&mut Env, &JObject) -> jni::errors::Result<T>) -> Result<T, String> {
    // android-activity sets up the singleton before `android_main` runs.
    let vm = JavaVM::singleton().map_err(|e| format!("no Java VM: {e}"))?;
    vm.attach_current_thread(|env| -> jni::errors::Result<T> {
        let raw = app.activity_as_ptr() as jni::sys::jobject;
        // SAFETY: `activity_as_ptr` is a JNI global reference to the activity, owned by
        // android-activity and valid for as long as `app` is (android-activity's documented
        // contract). It is borrowed, never wrapped in an owning `Global` or `Auto`, and the cast
        // does not outlive this closure, which holds a reference to `app`.
        let activity = unsafe { env.as_cast_raw::<Global<JObject>>(&raw)? };
        let result = f(env, activity.as_ref());
        if env.exception_check() {
            env.exception_clear();
        }
        result
    })
    .map_err(|e| e.to_string())
}
