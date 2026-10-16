// The Android shell around the Rust library `apps/pdfcraft-android` (see docs/android/README.md).
// Build with `cargo xtask android`, which builds the library into app/src/main/jniLibs first.
plugins {
    id("com.android.application") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
}
