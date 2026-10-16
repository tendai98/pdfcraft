//! The [`PlatformHost`](crate::platform_host::PlatformHost) implementations that live in the
//! shell. Android's lives in its app crate (`apps/pdfcraft-android`), next to its JNI bridge.

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub mod desktop;
