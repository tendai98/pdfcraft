# PdfCraft for Android

Status: Phases 0–2 of the porting specification in progress (2026-10-16). Builds a debug APK for
arm64-v8a and x86_64; not yet verified on a device (the development machine has no emulator
acceleration and no device attached), so every on-device acceptance item below is **unverified**.

## Build

```sh
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
export ANDROID_HOME=/path/to/sdk            # platform 35, build-tools 35.0.0
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/27.2.12479018   # optional: the newest NDK is found
cargo xtask android                          # both ABIs; --abi arm64-v8a for one
adb install android/app/build/outputs/apk/debug/app-debug.apk
```

`cargo xtask android` builds `apps/pdfcraft-android` with `cargo ndk` (release profile, stripped)
into `android/app/src/main/jniLibs/`, then runs Gradle. Gradle is fetched once into
`target/gradle/`, pinned by SHA-256, instead of committing a `gradle-wrapper.jar`; set
`PDFCRAFT_GRADLE` to use another. `--release` adds `assembleRelease` and `bundleRelease` (AAB),
signed only when `ANDROID_KEYSTORE_PATH`, `ANDROID_KEYSTORE_PASSWORD` and `ANDROID_KEY_ALIAS` are
set. No key is ever generated or committed.

Logs go to logcat with the tag `pdfcraft` (`adb logcat -s pdfcraft`); panics are logged there as
`panicked at …`.

## How it fits together

| Piece | Where | What it does |
|---|---|---|
| `PdfCraftActivity` | `android/app/src/main/java/…` (Kotlin) | A `NativeActivity`. Storage Access Framework pickers, `ACTION_VIEW`/`ACTION_SEND` intents, clipboard text, dark mode. A thin forwarder: it never calls into Rust. |
| `apps/pdfcraft-android` | Rust `cdylib` | `android_main` runs eframe with the shell's `PdfCraftApp`, gives it an `AndroidHost`, and polls the activity once a frame. |
| `PlatformHost` | `crates/ui-egui/src/platform_host.rs` | The seam: pickers, clipboard, publishing saves. `DesktopHost` (rfd, arboard) on desktops; `AndroidHost` on Android. |

### Files

Android gives apps content URIs, not paths, and the shell's pickers hand their follow-ups paths
(`pickers::Ask` → `then(app, Vec<PathBuf>)`). Rather than rewriting the 45 call sites to bytes,
the Android host *stages* documents:

- **Open / pick files**: the activity copies each chosen document into `cacheDir/saf/<n>/<name>`
  and answers with that path. The original URI and whether it is writable are remembered.
  Documents over 200 MB ask first; over 2 GB are refused, never read.
- **Save** (in place): the shell writes the staged path as on the desktop (validated first, commit
  `c1a41cc`), then `PlatformHost::after_write` has the activity copy it over the original URI.
  If the document came without write permission (a mail attachment, say), the save fails with
  "use Save As", and the document stays unsaved.
- **Save As / export**: `ACTION_CREATE_DOCUMENT`; the shell writes the staged path it was given,
  and the file is published after the write (`after_write`) or after the follow-up ran
  (`after_pick`).
- **Folder to write into** (Split, Extract to files, batch results): `ACTION_OPEN_DOCUMENT_TREE`;
  the shell gets an empty staged folder, and files it wrote there are created in the tree after
  the follow-up ran. Files a background worker writes *later* are not published yet (Phase 3).
- **Folder to read** (Combine ▸ Add folder, Pin folder to Home): not available yet; says so.

The activity sets `TMPDIR`, `HOME` and the `XDG_*` variables to its private folders before the
library loads, so the shell's existing Linux paths (settings, recovery, temp files) land in
`filesDir`/`cacheDir` without Android-specific code. eframe's settings file is `filesDir/app.ron`.

### Unsafe

`apps/pdfcraft-android` is the one crate with `unsafe_code = "deny"` instead of the workspace's
`forbid`, and `src/ffi.rs` the one module allowed it (approved by the owner, 2026-10-16): exporting
`android_main` needs `#[unsafe(no_mangle)]`, and borrowing the activity's JNI reference needs
`Env::as_cast_raw`. The Java VM comes from `JavaVM::singleton()` (safe). Because Rust polls the
activity instead of the activity calling Rust, no `Java_*` function is exported.

### Renderer

wgpu (Vulkan, with wgpu's GLES backend as its fallback). A file `filesDir/renderer` containing
`gl` starts egui's OpenGL renderer instead (`adb shell run-as app.pdfcraft.android sh -c 'echo gl > files/renderer'`).

## Decisions and deviations from the specification

- Owner decisions (2026-10-16): Kotlin helper approved; automation not compiled in; placeholder
  identity (`app.pdfcraft.android`, "PdfCraft"); the launcher icon is the owner's lion art
  (2026-10-16; contributor-original, credited to PdfCraft contributors), as an adaptive icon on its own teal; this session covers
  Phases 0–2; phone layout and PrintManager are decided later (printing is unchanged for now).
- **No Noto CJK.** The specification lists `NotoSansCJK*` as a fallback font; AGENTS.md §1.1 bans
  Noto CJK (Source Han rebranded), and AGENTS.md wins. Android CJK fallback is Droid Sans
  Fallback where an older device has it; Chinese UI text needs craft-fonts or shows boxes.
- **`PdfCraftApp::new()` is kept**, with a public `host` field defaulting to the desktop host,
  instead of `new(host)`: dozens of tests and both other entry points call `new()`.
- **The web build keeps its own byte-based flows** (no `WebHost` yet): `Dialog::to_rfd` feeds
  rfd's browser picker, unchanged in behaviour.
- **jni 0.22**, not 0.21: android-activity 0.6.1 already uses it (one copy in the tree).
- **Staged paths instead of bytes** at the picker seam (above).
- Not done yet: Back button, IME insets, pinch zoom, `trim_caches`, printing, the CI job, the
  emulator smoke test (Phases 3–6).
