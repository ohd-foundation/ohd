//! Phase 4f — relay data-plane end-to-end integration tests.
//!
//! This crate intentionally carries no code. It exists only as a home for
//! the cross-workspace integration test in `tests/`, which has to depend
//! simultaneously on `cord-agent`, `ohd-relay`, `ohd-relay-client`
//! (`responder`), `ohd-storage-core`, and `ohd-mcp-core` — a dependency
//! fan-out no existing crate can host.
//!
//! See `tests/relay_data_plane.rs`.
