//! SQLite-backed store, behind an [`r2d2`] pool so each axum handler gets
//! a fresh connection. CORD persists only users, data-source credentials
//! (encrypted), BYO model keys (encrypted), and chat history.

use crate::errors::{ApiError, ApiResult};
use crate::util::{new_ulid, now_iso};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::Serialize;

const MIGRATION: &str = include_str!("../migrations/001_initial.sql");

#[derive(Clone)]
pub struct Db {
    pool: Pool<SqliteConnectionManager>,
}

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub cord_user_ulid: String,
    pub oidc_issuer: String,
    pub oidc_subject: String,
    pub display_label: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DataSource {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub endpoint: String,
    pub rendezvous_id: Option<String>,
    pub relay_host: Option<String>,
    #[serde(skip)]
    pub enc_token: String,
    #[serde(skip)]
    pub cert_pin: Option<String>,
    pub scope_json: Option<String>,
    pub status: String,
    pub created_at: String,
    pub last_ok_at: Option<String>,
}

/// Fields a caller supplies when registering a new source. `enc_token` is
/// expected already sealed (see `crypto`).
pub struct NewSource {
    pub label: String,
    pub kind: String,
    pub endpoint: String,
    pub rendezvous_id: Option<String>,
    pub relay_host: Option<String>,
    pub enc_token: String,
    pub cert_pin: Option<String>,
    pub scope_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ByoKey {
    pub id: String,
    pub provider_kind: String,
    pub label: String,
    #[serde(skip)]
    pub enc_api_key: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Chat {
    pub id: String,
    pub source_id: String,
    pub model: String,
    pub title: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

impl Db {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let pool = Pool::new(SqliteConnectionManager::file(path))?;
        let db = Self { pool };
        db.migrate()?;
        Ok(db)
    }

    pub fn in_memory() -> anyhow::Result<Self> {
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::memory())?;
        let db = Self { pool };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.pool.get()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(MIGRATION)?;
        Ok(())
    }

    // --- users -------------------------------------------------------------

    /// Find the CORD user for an OIDC `(issuer, subject)` pair, minting one
    /// on first sight. Always bumps `last_seen_at`.
    pub fn upsert_user(
        &self,
        issuer: &str,
        subject: &str,
        label: Option<&str>,
    ) -> ApiResult<User> {
        let conn = self.pool.get()?;
        let now = now_iso();
        let existing: Option<String> = conn
            .query_row(
                "SELECT cord_user_ulid FROM users WHERE oidc_issuer = ?1 AND oidc_subject = ?2",
                params![issuer, subject],
                |r| r.get(0),
            )
            .ok();
        let ulid = match existing {
            Some(u) => {
                conn.execute(
                    "UPDATE users SET last_seen_at = ?1, display_label = COALESCE(?2, display_label)
                     WHERE cord_user_ulid = ?3",
                    params![now, label, u],
                )?;
                u
            }
            None => {
                let u = new_ulid();
                conn.execute(
                    "INSERT INTO users
                       (cord_user_ulid, oidc_issuer, oidc_subject, display_label, created_at, last_seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    params![u, issuer, subject, label, now],
                )?;
                u
            }
        };
        drop(conn);
        self.user(&ulid)
    }

    pub fn user(&self, ulid: &str) -> ApiResult<User> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT cord_user_ulid, oidc_issuer, oidc_subject, display_label, created_at, last_seen_at
             FROM users WHERE cord_user_ulid = ?1",
            params![ulid],
            |r| {
                Ok(User {
                    cord_user_ulid: r.get(0)?,
                    oidc_issuer: r.get(1)?,
                    oidc_subject: r.get(2)?,
                    display_label: r.get(3)?,
                    created_at: r.get(4)?,
                    last_seen_at: r.get(5)?,
                })
            },
        )
        .map_err(no_rows_is_not_found)
    }

    // --- data sources ------------------------------------------------------

    pub fn insert_source(&self, user: &str, s: NewSource) -> ApiResult<DataSource> {
        let conn = self.pool.get()?;
        let id = new_ulid();
        let now = now_iso();
        conn.execute(
            "INSERT INTO data_sources
               (id, cord_user_ulid, label, kind, endpoint, rendezvous_id, relay_host,
                enc_token, cert_pin, scope_json, status, created_at, last_ok_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'connected', ?11, ?11)",
            params![
                id, user, s.label, s.kind, s.endpoint, s.rendezvous_id, s.relay_host,
                s.enc_token, s.cert_pin, s.scope_json, now,
            ],
        )?;
        drop(conn);
        self.get_source(user, &id)
    }

    pub fn list_sources(&self, user: &str) -> ApiResult<Vec<DataSource>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, label, kind, endpoint, rendezvous_id, relay_host, enc_token,
                    cert_pin, scope_json, status, created_at, last_ok_at
             FROM data_sources WHERE cord_user_ulid = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![user], row_to_source)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_source(&self, user: &str, id: &str) -> ApiResult<DataSource> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT id, label, kind, endpoint, rendezvous_id, relay_host, enc_token,
                    cert_pin, scope_json, status, created_at, last_ok_at
             FROM data_sources WHERE cord_user_ulid = ?1 AND id = ?2",
            params![user, id],
            row_to_source,
        )
        .map_err(no_rows_is_not_found)
    }

    pub fn delete_source(&self, user: &str, id: &str) -> ApiResult<()> {
        let conn = self.pool.get()?;
        let n = conn.execute(
            "DELETE FROM data_sources WHERE cord_user_ulid = ?1 AND id = ?2",
            params![user, id],
        )?;
        if n == 0 {
            return Err(ApiError::NotFound);
        }
        Ok(())
    }

    /// Update a source's reachability status; `ok = true` also stamps
    /// `last_ok_at`.
    pub fn set_source_status(&self, user: &str, id: &str, status: &str, ok: bool) -> ApiResult<()> {
        let conn = self.pool.get()?;
        let now = now_iso();
        let n = if ok {
            conn.execute(
                "UPDATE data_sources SET status = ?1, last_ok_at = ?2
                 WHERE cord_user_ulid = ?3 AND id = ?4",
                params![status, now, user, id],
            )?
        } else {
            conn.execute(
                "UPDATE data_sources SET status = ?1
                 WHERE cord_user_ulid = ?2 AND id = ?3",
                params![status, user, id],
            )?
        };
        if n == 0 {
            return Err(ApiError::NotFound);
        }
        Ok(())
    }

    // --- BYO model keys ----------------------------------------------------

    pub fn insert_byo(
        &self,
        user: &str,
        provider_kind: &str,
        label: &str,
        enc_api_key: &str,
    ) -> ApiResult<ByoKey> {
        let conn = self.pool.get()?;
        let id = new_ulid();
        let now = now_iso();
        conn.execute(
            "INSERT INTO byo_keys (id, cord_user_ulid, provider_kind, label, enc_api_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, user, provider_kind, label, enc_api_key, now],
        )?;
        Ok(ByoKey {
            id,
            provider_kind: provider_kind.to_string(),
            label: label.to_string(),
            enc_api_key: enc_api_key.to_string(),
            created_at: now,
        })
    }

    pub fn list_byo(&self, user: &str) -> ApiResult<Vec<ByoKey>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, provider_kind, label, enc_api_key, created_at
             FROM byo_keys WHERE cord_user_ulid = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![user], |r| {
                Ok(ByoKey {
                    id: r.get(0)?,
                    provider_kind: r.get(1)?,
                    label: r.get(2)?,
                    enc_api_key: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn delete_byo(&self, user: &str, id: &str) -> ApiResult<()> {
        let conn = self.pool.get()?;
        let n = conn.execute(
            "DELETE FROM byo_keys WHERE cord_user_ulid = ?1 AND id = ?2",
            params![user, id],
        )?;
        if n == 0 {
            return Err(ApiError::NotFound);
        }
        Ok(())
    }

    // --- chats -------------------------------------------------------------

    pub fn insert_chat(&self, user: &str, source_id: &str, model: &str) -> ApiResult<Chat> {
        let conn = self.pool.get()?;
        let id = new_ulid();
        let now = now_iso();
        conn.execute(
            "INSERT INTO chats (id, cord_user_ulid, source_id, model, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?5)",
            params![id, user, source_id, model, now],
        )?;
        drop(conn);
        self.get_chat(user, &id)
    }

    pub fn list_chats(&self, user: &str) -> ApiResult<Vec<Chat>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, source_id, model, title, created_at, updated_at
             FROM chats WHERE cord_user_ulid = ?1 ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map(params![user], row_to_chat)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_chat(&self, user: &str, id: &str) -> ApiResult<Chat> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT id, source_id, model, title, created_at, updated_at
             FROM chats WHERE cord_user_ulid = ?1 AND id = ?2",
            params![user, id],
            row_to_chat,
        )
        .map_err(no_rows_is_not_found)
    }

    pub fn delete_chat(&self, user: &str, id: &str) -> ApiResult<()> {
        // Ensure ownership before nuking messages — and before taking a
        // pooled connection, so the two never contend.
        self.get_chat(user, id)?;
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM chat_messages WHERE chat_id = ?1", params![id])?;
        conn.execute("DELETE FROM chats WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn insert_message(&self, chat_id: &str, role: &str, content: &str) -> ApiResult<ChatMessage> {
        let conn = self.pool.get()?;
        let id = new_ulid();
        let now = now_iso();
        conn.execute(
            "INSERT INTO chat_messages (id, chat_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, chat_id, role, content, now],
        )?;
        conn.execute(
            "UPDATE chats SET updated_at = ?1 WHERE id = ?2",
            params![now, chat_id],
        )?;
        Ok(ChatMessage {
            id,
            role: role.to_string(),
            content: content.to_string(),
            created_at: now,
        })
    }

    pub fn list_messages(&self, chat_id: &str) -> ApiResult<Vec<ChatMessage>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, role, content, created_at FROM chat_messages
             WHERE chat_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map(params![chat_id], |r| {
                Ok(ChatMessage {
                    id: r.get(0)?,
                    role: r.get(1)?,
                    content: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

fn row_to_source(r: &rusqlite::Row) -> rusqlite::Result<DataSource> {
    Ok(DataSource {
        id: r.get(0)?,
        label: r.get(1)?,
        kind: r.get(2)?,
        endpoint: r.get(3)?,
        rendezvous_id: r.get(4)?,
        relay_host: r.get(5)?,
        enc_token: r.get(6)?,
        cert_pin: r.get(7)?,
        scope_json: r.get(8)?,
        status: r.get(9)?,
        created_at: r.get(10)?,
        last_ok_at: r.get(11)?,
    })
}

fn row_to_chat(r: &rusqlite::Row) -> rusqlite::Result<Chat> {
    Ok(Chat {
        id: r.get(0)?,
        source_id: r.get(1)?,
        model: r.get(2)?,
        title: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

fn no_rows_is_not_found(e: rusqlite::Error) -> ApiError {
    match e {
        rusqlite::Error::QueryReturnedNoRows => ApiError::NotFound,
        other => ApiError::Db(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_upsert_is_idempotent() {
        let db = Db::in_memory().unwrap();
        let a = db.upsert_user("https://idp", "sub-1", Some("Jakub")).unwrap();
        let b = db.upsert_user("https://idp", "sub-1", None).unwrap();
        assert_eq!(a.cord_user_ulid, b.cord_user_ulid);
        assert_eq!(b.display_label.as_deref(), Some("Jakub"));
    }

    #[test]
    fn source_round_trip_and_scoping() {
        let db = Db::in_memory().unwrap();
        let u = db.upsert_user("https://idp", "s", None).unwrap();
        let other = db.upsert_user("https://idp", "other", None).unwrap();
        let src = db
            .insert_source(
                &u.cord_user_ulid,
                NewSource {
                    label: "My phone".into(),
                    kind: "direct".into(),
                    endpoint: "https://storage.example".into(),
                    rendezvous_id: None,
                    relay_host: None,
                    enc_token: "sealed".into(),
                    cert_pin: None,
                    scope_json: None,
                },
            )
            .unwrap();
        assert!(db.get_source(&u.cord_user_ulid, &src.id).is_ok());
        // A different user cannot see it.
        assert!(matches!(
            db.get_source(&other.cord_user_ulid, &src.id),
            Err(ApiError::NotFound)
        ));
    }
}
