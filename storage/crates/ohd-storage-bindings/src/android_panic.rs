//! Routes Rust panics — and `tracing`/`log` events — to Android logcat.
//!
//! Rust's default panic handler writes to stderr, which Android discards —
//! so a panic on a background thread (notably the share-responder tokio
//! runtime) leaves no trace in `adb logcat`, only an opaque `SIGABRT`.
//! [`install`] installs `android_logger` as the `log` backend and adds a
//! panic hook that emits the panic location + message through `log::error!`,
//! then chains to the previous hook.
//!
//! Because `tracing` is built with its `log` feature, every `tracing` event
//! in the dependency tree (the relay tunnel client + share responder log
//! through `tracing`) also reaches this `log` backend — so the whole
//! tunnel-client lifecycle is visible under `adb logcat -s OhdRust`.
//!
//! This crate is `#![forbid(unsafe_code)]`; the `liblog` FFI lives inside
//! `android_logger`, not here.

use std::sync::Once;

static INSTALLED: Once = Once::new();

/// Install the logcat panic hook + log backend. Idempotent and cheap — call
/// it at the entry of any FFI export whose work may panic / log off the main
/// thread.
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
            .with_tag("OhdRust")
            // Debug so the relay tunnel client's reconnect / heartbeat /
            // close events (logged at debug!) are captured — they are the
            // diagnostics for tunnel-stability work.
            .with_max_level(log::LevelFilter::Debug)
            // Prefix each line with the record target (the `tracing` target,
            // e.g. `ohd_relay_client::tunnel`) so a single logcat tag still
            // tells you which subsystem emitted the line.
            .format(|f, record| write!(f, "[{}] {}", record.target(), record.args())),
    );
}

/// On non-Android hosts there is no logcat; `log::error!` is an inert
/// no-op with no logger installed, so the panic hook still chains safely.
#[cfg(not(target_os = "android"))]
fn init_backend() {}
