//! Standalone `uniffi-bindgen` binary.
//!
//! Generates Kotlin / Swift / Python source files from this crate's exposed
//! uniffi metadata. From the workspace root invoke as:
//!
//! ```text
//! cargo run -p ohd-storage-bindings --features cli --bin uniffi-bindgen -- \
//!     generate \
//!     --library storage/target/debug/libohd_storage_bindings.so \
//!     --language kotlin \
//!     --out-dir connect/android/app/src/main/java
//! ```
//!
//! Two gotchas worth knowing:
//!  - **Use the debug `.so`, not release.** The release profile strips the
//!    uniffi metadata symbols `--library` mode reads, so a release-built
//!    cdylib silently produces zero output (exit 0, no files written).
//!    Build with `cargo build -p ohd-storage-bindings` (debug) first.
//!  - **`--out-dir` is the Kotlin source root** (`.../java`), not the
//!    `uniffi/` package directory. The bindgen appends `uniffi/<namespace>/`
//!    itself, so pointing at `.../java/uniffi` writes a double-`uniffi/`
//!    path that the app never sees.
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
