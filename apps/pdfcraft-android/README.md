# pdfcraft-android

PdfCraft for Android: the egui shell (`pdfcraft-ui-egui`) in a NativeActivity, with the Storage
Access Framework behind the shell's `PlatformHost`. Build it with `cargo xtask android`; it is not
a default workspace member and builds only for `target_os = "android"`. On other targets the
library is empty, apart from the event protocol and its tests (`cargo test -p pdfcraft-android`).

| Module | Role |
|---|---|
| `ffi.rs` | the only `unsafe` in PdfCraft: the `android_main` export and borrowing the activity's JNI reference (owner-approved, 2026-10-16) |
| `bridge.rs` | safe JNI calls to `PdfCraftActivity` (`android/app/src/main/java/…`) |
| `events.rs` | the activity's event lines, parsed as untrusted input |
| `host.rs` | `AndroidHost`: pickers by staging documents in the cache, publishing writes back |
| `shell.rs` | `Shell`: the `eframe::App` around `PdfCraftApp` (theme, keyboard, clipboard) |

Lints: the workspace's, except `unsafe_code = "deny"` instead of `forbid`, so `ffi.rs` alone can
allow it. Everything else follows AGENTS.md §4: no panics, untrusted input capped.

See [docs/android/README.md](../../docs/android/README.md) for the design, the bridge protocol
and the device checklist.
