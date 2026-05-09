//! Standalone `uniffi-bindgen` binary.
//!
//! Generates Kotlin / Swift / Python source files from this crate's exposed
//! uniffi metadata. Invoke as:
//!
//! ```text
//! cargo run --features cli --bin uniffi-bindgen -- generate \
//!     --library target/release/libohd_storage_bindings.so \
//!     --language kotlin \
//!     --out-dir ../../../connect/android/app/src/main/java/uniffi
//! ```
//!
//! `--library` mode reads metadata directly out of the compiled cdylib,
//! sidestepping the need for a separate `.udl` file (we use uniffi 0.28's
//! proc-macro mode — see `lib.rs::setup_scaffolding!`).
//!
//! See `connect/android/BUILD.md` and `connect/ios/BUILD.md` for the
//! end-to-end build recipes.

#[cfg(feature = "cli")]
fn main() {
    uniffi::uniffi_bindgen_main()
}

#[cfg(not(feature = "cli"))]
fn main() {
    eprintln!(
        "uniffi-bindgen requires the `cli` feature. Re-run with:\n\
         \n\
         \tcargo run --features cli --bin uniffi-bindgen -- <args>\n\
         \n\
         The cli feature pulls in clap/camino — kept off the default build to\n\
         keep `cargo build --workspace` light."
    );
    std::process::exit(2);
}
