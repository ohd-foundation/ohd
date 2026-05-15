//! Stdio transport — what Claude Code, Claude Desktop, Cursor and
//! Codex use locally. One JSON-RPC message per line on stdin → response
//! per line on stdout. stderr is the log channel.

use crate::dispatch::dispatch;
use crate::jsonrpc::{Request, Response, PARSE_ERROR};
use ohd_storage_core::Storage;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub async fn run(storage: Arc<Storage>) -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();

    while let Some(line) = reader.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(trimmed) {
            Ok(req) => dispatch(&req, &storage),
            Err(e) => Response::err(None, PARSE_ERROR, format!("parse error: {e}")),
        };
        // Notifications (no id) per JSON-RPC 2.0 should produce no
        // output, but we tolerate ill-behaved clients by always
        // responding when there's an id.
        if response.id.is_some() || response.error.is_some() {
            let mut bytes = serde_json::to_vec(&response)?;
            bytes.push(b'\n');
            stdout.write_all(&bytes).await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}
