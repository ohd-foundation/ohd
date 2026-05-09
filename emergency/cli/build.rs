//! Build-time codegen for the OHDC Connect-RPC client used by
//! `ohd-emergency case-export` and (once the storage RPC lands) `audit`.
//!
//! Mirrors `../../connect/cli/build.rs` so the emitted client is wire-
//! compatible with the storage server's generated service trait. The proto
//! root lives in the storage component (`../../storage/proto/`); we resolve
//! that relative to `CARGO_MANIFEST_DIR` at build time so the path stays
//! correct whether the CLI is built from the workspace, from
//! `cargo install --path`, or from a source archive that preserves the
//! repo layout.
//!
//! Output: `$OUT_DIR/_connectrpc.rs`, included by `src/main.rs` via the
//! `connectrpc::include_generated!()` macro.

fn main() {
    // Use the vendored protoc binary so the build doesn't depend on a
    // system protoc install. `connectrpc-build` reads the `PROTOC` env var.
    let protoc = protoc_bin_vendored::protoc_bin_path()
        .expect("protoc-bin-vendored should ship a protoc binary for this host");
    // SAFETY: build.rs is single-threaded; setting an env var here is the
    // standard prost-build / connectrpc-build pattern for the vendored protoc.
    unsafe {
        std::env::set_var("PROTOC", &protoc);
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set");
    // emergency/cli → emergency → ohd → storage/proto
    let proto_root = std::path::PathBuf::from(&manifest_dir)
        .parent() // emergency/
        .and_then(|p| p.parent()) // ohd/
        .map(|p| p.join("storage").join("proto"))
        .expect("crate is at <ohd>/emergency/cli");

    let ohdc_proto = proto_root.join("ohdc/v0/ohdc.proto");
    assert!(
        ohdc_proto.exists(),
        "expected ohdc.proto at {}",
        ohdc_proto.display()
    );

    connectrpc_build::Config::new()
        .files(&[ohdc_proto])
        .includes(&[proto_root.clone()])
        .include_file("_connectrpc.rs")
        .compile()
        .expect("connectrpc-build codegen failed");

    println!("cargo:rerun-if-changed={}", proto_root.display());
}
