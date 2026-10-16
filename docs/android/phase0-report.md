# Android Phase 0: does the workspace build for Android?

Status: done (2026-10-16). Every engine crate builds for `aarch64-linux-android` with no source
change; the shell needed the dependency gates the specification predicted, plus one it didn't
(eframe's `accesskit` feature).

Toolchain: Rust 1.99.1, cargo-ndk 4, NDK r27c (27.2.12479018), API level 26, on Linux x86_64 with
two cores. Source: `c1a41cc` (`a60d5d1` with the specification added).

## Method, and where it differs from the plan

The plan was one `cargo ndk -t arm64-v8a build -p <crate>` per crate. That run was stopped
partway through `pdfcraft-engine`'s dependency tree, by the owner's choice, to save time on a
two-core machine: building each crate separately rebuilt little that the app build doesn't. The
results below therefore come from:

1. the stopped per-crate run (unchanged source, dev profile): everything up to `boa_engine`
   compiled without an error;
2. `cargo ndk -t arm64-v8a check -p pdfcraft-android`, which checks every crate below;
3. the release build of `pdfcraft-android` (`cargo xtask android --abi arm64-v8a`), which
   compiled and linked all of them into `libpdfcraft_android.so`.

The shell's rfd and arboard failures were diagnosed from their sources (neither has an Android
backend: rfd's `backend.rs`, arboard's `platform/mod.rs`) and gated before the shell was first
compiled for Android, so they were not observed as compiler errors.

## Results

| Crate | Result | Error class / note |
|---|---|---|
| `pdfcraft-engine` and every engine crate it uses (`cos`, `filters`, `crypt`, `model`, `content`, `fonts`, `edit`, `forms`, `xfa`, `annot`, `organize`, `redact`, `a11y`, `audit`, `compare`, `create`, `export`, `measure`, `optimize`, `preflight`, `xfdf`, `geom`) | builds | none |
| `pdfcraft-render` (hayro, rayon work queue) | builds | none |
| `pdfcraft-ocr` (ocrs, rten) | builds | none; ships an unused `cdylib` too (below) |
| `pdfcraft-sign` | builds | none; the tree's `aws-lc-sys` (C) builds with NDK r27 clang |
| `pdfcraft-js` (boa) | builds | none; boa_engine ships an unused `cdylib` too |
| `pdfcraft-print` | builds | none (the spooler is unused on Android; Phase 3) |
| `pdfcraft-platform` | builds | none |
| `pdfcraft-ui-egui` | fails unchanged | **dependency without an Android backend**: rfd, arboard (from source). **eframe feature**: `accesskit` "is only available with `android-game-activity`" (observed). |

Fixes (Phase 2a, `crates/ui-egui/Cargo.toml`): rfd and arboard move under
`cfg(not(any(target_arch = "wasm32", target_os = "android")))` behind the new `PlatformHost`
seam; eframe's `accesskit` feature moves under `cfg(not(target_os = "android"))`. AccessKit on
Android is deferred anyway (specification, "Out of scope"). No engine crate needed a feature flag
or a patch, so nothing here blocks Phase 1.

## Findings for later phases

- **Library size: 47.3 MB stripped** for arm64-v8a (release, thin LTO, `codegen-units = 1`),
  above the 40 MB budget. Recorded for the owner (specification, "Performance budgets").
- `boa_engine`, `ocrs` and `rten` declare a `cdylib` crate type, so cargo-ndk copies three unused
  `.so` files next to the app's; `cargo xtask android` deletes them. `libpdfcraft_android.so`
  needs only `liblog`, `libdl`, `libandroid`, `libm` and `libc`.
- The specification says the tree is `aws-lc-sys`-free; it isn't, but `aws-lc-sys` builds with
  NDK r27 without extra flags.
- `nm -D libpdfcraft_android.so` has no rfd or arboard symbol (the only match for "rfd" is libc's
  `dirfd`).
