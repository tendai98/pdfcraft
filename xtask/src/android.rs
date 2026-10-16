//! `cargo xtask android`: build the Android app (docs/android/README.md).
//!
//! 1. `cargo ndk` builds `apps/pdfcraft-android` for each ABI into `android/app/src/main/jniLibs/`
//!    (always the release profile: render speed matters more than build time).
//! 2. Gradle packages it with the Kotlin activity: a debug-signed APK by default, or with
//!    `--release` a release APK and AAB (signed when `ANDROID_KEYSTORE_PATH`,
//!    `ANDROID_KEYSTORE_PASSWORD` and `ANDROID_KEY_ALIAS` are set; never a committed key).
//!
//! Needs the Android SDK (`ANDROID_HOME`), an NDK (`ANDROID_NDK_HOME`, or the newest under
//! `$ANDROID_HOME/ndk`), `cargo-ndk`, the Rust targets and a JDK 17+. Gradle itself is fetched
//! once into `target/gradle/`, pinned by SHA-256, so no wrapper jar is committed.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const GRADLE_VERSION: &str = "8.10.2";
/// SHA-256 of gradle-8.10.2-bin.zip, as published at services.gradle.org.
const GRADLE_SHA256: &str = "31c55713e40233a8303827ceb42ca48a47267a0ad4bab9177123121e71524c26";
/// The ABIs built by default: phones and tablets, and the x86_64 emulator.
const DEFAULT_ABIS: &[&str] = &["arm64-v8a", "x86_64"];
/// minSdk (android/app/build.gradle.kts).
const MIN_SDK: &str = "26";

fn usage() -> &'static str {
    "usage: cargo xtask android [--release] [--abi arm64-v8a|x86_64]... [--no-gradle]\n\
     \n\
     --release    also build the release APK and AAB (signed when the ANDROID_KEYSTORE_* variables are set)\n\
     --abi ABI    build only this ABI (repeatable; default: arm64-v8a and x86_64)\n\
     --no-gradle  build the Rust library only"
}

pub fn run(args: &[String]) -> Result<()> {
    let mut release = false;
    let mut gradle = true;
    let mut abis: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--release" => release = true,
            "--no-gradle" => gradle = false,
            "--abi" => match it.next() {
                Some(abi) if DEFAULT_ABIS.contains(&abi.as_str()) => abis.push(abi.clone()),
                Some(abi) => bail!("unsupported ABI {abi} (arm64-v8a or x86_64)\n{}", usage()),
                None => bail!("--abi needs a value\n{}", usage()),
            },
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(());
            }
            other => bail!("unknown argument {other}\n{}", usage()),
        }
    }
    if abis.is_empty() {
        abis = DEFAULT_ABIS.iter().map(|s| s.to_string()).collect();
    }

    let root = crate::gates::root();
    let sdk = sdk_dir()?;
    let ndk = ndk_dir(&sdk)?;
    check_tools(&abis)?;

    let jni_libs = root.join("android/app/src/main/jniLibs");
    let mut cargo = Command::new("cargo");
    cargo.current_dir(&root).env("ANDROID_NDK_HOME", &ndk).env("ANDROID_HOME", &sdk).arg("ndk");
    for abi in &abis {
        cargo.args(["-t", abi]);
    }
    cargo
        .args(["--platform", MIN_SDK, "-o"])
        .arg(&jni_libs)
        .args(["build", "-p", "pdfcraft-android", "--release"])
        // The library is the bulk of the APK; symbols stay out of it (spec: ≤ 40 MB per ABI).
        .env("CARGO_PROFILE_RELEASE_STRIP", "true");
    println!("android: building the Rust library for {}", abis.join(", "));
    let status = cargo.status().context("running cargo ndk (install it with `cargo install cargo-ndk`)")?;
    if !status.success() {
        bail!("cargo ndk failed");
    }
    for abi in &abis {
        // cargo ndk copies every shared library the build produced, including the unused cdylib
        // outputs some dependencies declare (boa_engine, ocrs, rten): ship only the app's.
        for entry in std::fs::read_dir(jni_libs.join(abi))?.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "so") && path.file_name().is_some_and(|n| n != "libpdfcraft_android.so") {
                std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            }
        }
        let lib = jni_libs.join(abi).join("libpdfcraft_android.so");
        let size = std::fs::metadata(&lib).with_context(|| format!("{} was not built", lib.display()))?.len();
        println!("android: {} ({:.1} MB)", lib.display(), size as f64 / 1_048_576.0);
    }
    if !gradle {
        return Ok(());
    }

    let gradle = gradle_bin(&root)?;
    let mut tasks = vec!["assembleDebug"];
    if release {
        tasks.extend(["assembleRelease", "bundleRelease"]);
    }
    println!("android: gradle {}", tasks.join(" "));
    let status = Command::new(&gradle)
        .current_dir(root.join("android"))
        .env("ANDROID_HOME", &sdk)
        .args(["--no-daemon", "--console=plain"])
        .args(&tasks)
        .status()
        .with_context(|| format!("running {}", gradle.display()))?;
    if !status.success() {
        bail!("gradle failed");
    }
    let outputs = root.join("android/app/build/outputs");
    for apk in [
        outputs.join("apk/debug/app-debug.apk"),
        outputs.join("apk/release/app-release.apk"),
        outputs.join("apk/release/app-release-unsigned.apk"),
        outputs.join("bundle/release/app-release.aab"),
    ] {
        if let Ok(m) = std::fs::metadata(&apk) {
            println!("android: {} ({:.1} MB)", apk.display(), m.len() as f64 / 1_048_576.0);
        }
    }
    Ok(())
}

fn sdk_dir() -> Result<PathBuf> {
    let dir = std::env::var_os("ANDROID_HOME")
        .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
        .map(PathBuf::from)
        .context("set ANDROID_HOME to the Android SDK (platform 35, build-tools 35.0.0)")?;
    if !dir.join("platforms").is_dir() {
        bail!("{} doesn't look like an Android SDK (no platforms/)", dir.display());
    }
    Ok(dir)
}

/// `ANDROID_NDK_HOME`, or the newest NDK installed in the SDK.
fn ndk_dir(sdk: &Path) -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("ANDROID_NDK_HOME").map(PathBuf::from) {
        return Ok(dir);
    }
    let mut versions: Vec<PathBuf> = std::fs::read_dir(sdk.join("ndk"))
        .with_context(|| format!("no NDK: set ANDROID_NDK_HOME or install one under {}/ndk (r27 or newer)", sdk.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    // NDK folders are named by version (27.2.12479018): newest last when compared numerically.
    versions.sort_by_key(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.split('.').map(|part| part.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>())
            .unwrap_or_default()
    });
    versions.pop().context("no NDK found: set ANDROID_NDK_HOME (r27 or newer)")
}

fn check_tools(abis: &[String]) -> Result<()> {
    let ndk = Command::new("cargo").args(["ndk", "--version"]).output();
    if !ndk.is_ok_and(|o| o.status.success()) {
        bail!("cargo-ndk is missing: cargo install cargo-ndk");
    }
    // Without rustup (a distribution's Rust), cargo reports a missing target itself.
    if let Ok(out) = Command::new("rustup").args(["target", "list", "--installed"]).output() {
        let installed = String::from_utf8_lossy(&out.stdout);
        for abi in abis {
            let target = if abi == "x86_64" { "x86_64-linux-android" } else { "aarch64-linux-android" };
            if !installed.lines().any(|l| l.trim() == target) {
                bail!("the Rust target {target} is missing: rustup target add {target}");
            }
        }
    }
    Ok(())
}

/// Gradle from `PDFCRAFT_GRADLE`, or the pinned distribution under `target/gradle/`.
fn gradle_bin(root: &Path) -> Result<PathBuf> {
    if let Some(g) = std::env::var_os("PDFCRAFT_GRADLE") {
        return Ok(PathBuf::from(g));
    }
    let dir = root.join("target/gradle");
    let bin = dir.join(format!("gradle-{GRADLE_VERSION}/bin/gradle"));
    if bin.is_file() {
        return Ok(bin);
    }
    std::fs::create_dir_all(&dir)?;
    let zip = dir.join(format!("gradle-{GRADLE_VERSION}-bin.zip"));
    crate::assets::fetch_verified(&format!("https://services.gradle.org/distributions/gradle-{GRADLE_VERSION}-bin.zip"), &zip, GRADLE_SHA256)?;
    let status = Command::new("unzip").args(["-q", "-o"]).arg(&zip).arg("-d").arg(&dir).status().context("running unzip")?;
    if !status.success() || !bin.is_file() {
        bail!("couldn't unpack {}", zip.display());
    }
    Ok(bin)
}
