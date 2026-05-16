//! Routes Rust panics to Android logcat.
//!
//! Rust's default panic handler writes to stderr, which Android discards —
//! so a panic on a background thread (notably the share-responder tokio
//! runtime) leaves no trace in `adb logcat`, only an opaque `SIGABRT`.
//! [`install`] installs `android_logger` as the `log` backend and adds a
//! panic hook that emits the panic location + message through `log::error!`
//! (logcat tag `OhdRustPanic`), then chains to the previous hook.
//!
//! This crate is `#![forbid(unsafe_code)]`; the `liblog` FFI lives inside
//! `android_logger`, not here.

use std::sync::Once;

static INSTALLED: Once = Once::new();

/// Install the logcat panic hook. Idempotent and cheap — call it at the
/// entry of any FFI export whose work may panic off the main thread.
pub fn install() {
    INSTALLED.call_once(|| {
        init_backend();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // `info` Display is `panicked at <file>:<line>:<col>:\n<message>`.
            log::error!("{info}");
            prev(info);
        }));
    });
}

#[cfg(target_os = "android")]
fn init_backend() {
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("OhdRustPanic")
            .with_max_level(log::LevelFilter::Error),
    );
}

/// On non-Android hosts there is no logcat; `log::error!` is an inert
/// no-op with no logger installed, so the panic hook still chains safely.
#[cfg(not(target_os = "android"))]
fn init_backend() {}
