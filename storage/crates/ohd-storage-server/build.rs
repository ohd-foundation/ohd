//! Build-time codegen for the OHDC Connect-RPC service.
//!
//! Compiles `proto/ohdc/v0/ohdc.proto` into `$OUT_DIR/_connectrpc.rs` via
//! `connectrpc-build`. The output contains:
//!   - buffa-emitted message types (Owned / View / serde JSON helpers)
//!   - `pub trait OhdcService` + `OhdcServiceExt` (server-side trait the
//!     business-logic adapter implements)
//!   - `pub struct OhdcServiceClient<T>` (client used by the e2e test)
//!
//! The other three protos (`auth.proto`, `relay.proto`, `sync.proto`) are
//! intentionally NOT compiled here — their service handlers are deferred per
//! STATUS.md, and the `Auth*Request` / `Relay*Request` types they declare
//! aren't referenced by anything in `ohd-storage-server` yet. Adding them is
//! a one-line `.files(...)` extension once the corresponding handlers land.

fn main() {
    // Use the vendored protoc binary so the build doesn't depend on a system
    // protoc install. `connectrpc-build` reads the `PROTOC` env var.
    let protoc = protoc_bin_vendored::protoc_bin_path()
        .expect("protoc-bin-vendored should ship a protoc binary for this host");
    // Safety: setting an env var in build.rs is process-local and standard
    // practice (prost-build does the same). The unsafe block is required by
    // edition 2024.
    // SAFETY: build.rs is single-threaded.
    unsafe {
        std::env::set_var("PROTOC", &protoc);
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set");
    let proto_root = std::path::PathBuf::from(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .expect("crate is at storage/crates/ohd-storage-server")
        .join("proto");

    connectrpc_build::Config::new()
        .files(&[
            proto_root.join("ohdc/v0/ohdc.proto"),
            proto_root.join("ohdc/v0/sync.proto"),
            // auth.proto is compiled now that the multi-identity account-
            // linking handlers (ListIdentities / LinkIdentityStart /
            // CompleteIdentityLink / UnlinkIdentity / SetPrimaryIdentity)
            // are wired in `auth_server.rs`. The other AuthService RPCs
            // (sessions, invites, device tokens, notifications) compile as
            // generated trait methods that return Unimplemented from the
            // adapter — the wire surface is still useful as a discoverable
            // contract.
            proto_root.join("ohdc/v0/auth.proto"),
        ])
        .includes(&[proto_root.clone()])
        .include_file("_connectrpc.rs")
        .compile()
        .expect("connectrpc-build codegen failed");

    println!("cargo:rerun-if-changed={}", proto_root.display());
}
