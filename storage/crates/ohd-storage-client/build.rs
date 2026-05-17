//! Build-time codegen for the OHDC ConnectRPC *client*.
//!
//! Compiles `proto/ohdc/v0/ohdc.proto` into `$OUT_DIR/_connectrpc.rs` via
//! `connectrpc-build`. The output contains:
//!   - buffa-emitted message types (Owned / View / serde JSON helpers)
//!   - `pub struct OhdcServiceClient<T>` — the generic ConnectRPC client this
//!     crate wraps in `OhdcRemoteClient`.
//!
//! Only `ohdc.proto` is compiled — the remote client surface is the
//! `OhdcService` consumer API. `auth.proto` / `sync.proto` carry separate
//! services the Android binding doesn't drive remotely in Phase 1.

fn main() {
    // Vendored protoc so the build doesn't depend on a system install.
    // `connectrpc-build` reads the `PROTOC` env var.
    let protoc = protoc_bin_vendored::protoc_bin_path()
        .expect("protoc-bin-vendored should ship a protoc binary for this host");
    // SAFETY: build.rs is single-threaded; setting a process-local env var
    // here is standard practice (prost-build / connectrpc-build do the same).
    unsafe {
        std::env::set_var("PROTOC", &protoc);
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set");
    let proto_root = std::path::PathBuf::from(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .expect("crate is at storage/crates/ohd-storage-client")
        .join("proto");

    connectrpc_build::Config::new()
        .files(&[proto_root.join("ohdc/v0/ohdc.proto")])
        .includes(&[proto_root.clone()])
        .include_file("_connectrpc.rs")
        .compile()
        .expect("connectrpc-build codegen failed");

    println!("cargo:rerun-if-changed={}", proto_root.display());
}
