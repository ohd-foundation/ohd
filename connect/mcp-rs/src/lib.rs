//! OHD MCP server — thin transport over `ohd-mcp-core`.

pub mod dispatch;
pub mod jsonrpc;
pub mod http;
pub mod stdio;

use ohd_storage_core::{Storage, StorageConfig};
use std::path::PathBuf;

/// Open the shared storage handle on startup. Single-user for v1; the
/// per-profile multi-tenant story tracks against the SaaS service that
/// already mints HS256 JWTs (`profile_ulid` → storage path lookup).
pub fn open_storage(path: PathBuf) -> anyhow::Result<Storage> {
    let cfg = StorageConfig::new(path);
    Ok(Storage::open(cfg)?)
}
