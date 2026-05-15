//! SQLite-backed account store. Held in an [`r2d2`] pool so axum handlers
//! get a fresh connection per request without churning sqlite handles.

use crate::errors::{ApiError, ApiResult};
use crate::plans::Plan;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};

const MIGRATION: &str = include_str!("../migrations/001_initial.sql");

#[derive(Clone)]
pub struct Db {
    pool: Pool<SqliteConnectionManager>,
}

#[derive(Debug, Serialize)]
pub struct Profile {
    pub profile_ulid: String,
    pub plan: Plan,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct OidcLink {
    pub provider: String,
    pub sub: String,
    pub display_label: Option<String>,
    pub linked_at: String,
}

#[derive(Debug, Serialize)]
pub struct Payment {
    pub ulid: String,
    pub created_at: String,
    pub amount_minor_units: i64,
    pub currency: String,
    pub provider: String,
    pub provider_charge_id: Option<String>,
    pub status: String,
    pub invoice_url: Option<String>,
}

impl Db {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let manager = SqliteConnectionManager::file(path);
        let pool = Pool::new(manager)?;
        Ok(Self { pool })
    }

    /// In-memory database — for tests.
    pub fn in_memory() -> anyhow::Result<Self> {
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder().max_size(1).build(manager)?;
        let db = Self { pool };
        db.migrate()?;
        Ok(db)
    }

    pub fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.pool.get()?;
        conn.execute_batch(MIGRATION)?;
        Ok(())
    }

    /// Create or claim a profile. If the (profile_ulid, recovery_hash) pair
    /// already exists we just bump `last_seen_at` and return the same row.
    /// If `profile_ulid` exists with a different recovery hash → conflict.
    pub fn register_profile(
        &self,
        profile_ulid: &str,
        recovery_code: &str,
        now_iso: &str,
    ) -> ApiResult<Profile> {
        let hash = hash_recovery(recovery_code);
        let conn = self.pool.get()?;
        let existing: Option<(String, String)> = conn
            .query_row(
                "SELECT recovery_hash_hex, plan FROM profiles WHERE profile_ulid = ?1",
                params![profile_ulid],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .ok();
        if let Some((existing_hash, plan)) = existing {
            if existing_hash != hash {
                return Err(ApiError::Conflict);
            }
            conn.execute(
                "UPDATE profiles SET last_seen_at = ?1 WHERE profile_ulid = ?2",
                params![now_iso, profile_ulid],
            )?;
            return Ok(Profile {
                profile_ulid: profile_ulid.to_string(),
                plan: Plan::from_db_str(&plan),
                created_at: now_iso.to_string(),
            });
        }
        conn.execute(
            "INSERT INTO profiles (profile_ulid, recovery_hash_hex, plan, created_at, last_seen_at)
             VALUES (?1, ?2, 'free', ?3, ?3)",
            params![profile_ulid, hash, now_iso],
        )?;
        Ok(Profile {
            profile_ulid: profile_ulid.to_string(),
            plan: Plan::Free,
            created_at: now_iso.to_string(),
        })
    }

    pub fn lookup_by_recovery(&self, recovery_code: &str) -> ApiResult<Profile> {
        let hash = hash_recovery(recovery_code);
        let conn = self.pool.get()?;
        let row = conn.query_row(
            "SELECT profile_ulid, plan, created_at FROM profiles WHERE recovery_hash_hex = ?1",
            params![hash],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        );
        match row {
            Ok((ulid, plan, created)) => Ok(Profile {
                profile_ulid: ulid,
                plan: Plan::from_db_str(&plan),
                created_at: created,
            }),
            Err(rusqlite::Error::QueryReturnedNoRows) => Err(ApiError::NotFound),
            Err(e) => Err(e.into()),
        }
    }

    pub fn lookup_by_oidc(&self, provider: &str, sub: &str) -> ApiResult<Profile> {
        let conn = self.pool.get()?;
        let row = conn.query_row(
            "SELECT p.profile_ulid, p.plan, p.created_at
             FROM oidc_identities o
             JOIN profiles p ON p.profile_ulid = o.profile_ulid
             WHERE o.provider = ?1 AND o.sub = ?2",
            params![provider, sub],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        );
        match row {
            Ok((ulid, plan, created)) => Ok(Profile {
                profile_ulid: ulid,
                plan: Plan::from_db_str(&plan),
                created_at: created,
            }),
            Err(rusqlite::Error::QueryReturnedNoRows) => Err(ApiError::NotFound),
            Err(e) => Err(e.into()),
        }
    }

    pub fn profile(&self, profile_ulid: &str) -> ApiResult<Profile> {
        let conn = self.pool.get()?;
        let row = conn.query_row(
            "SELECT plan, created_at FROM profiles WHERE profile_ulid = ?1",
            params![profile_ulid],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        );
        match row {
            Ok((plan, created)) => Ok(Profile {
                profile_ulid: profile_ulid.to_string(),
                plan: Plan::from_db_str(&plan),
                created_at: created,
            }),
            Err(rusqlite::Error::QueryReturnedNoRows) => Err(ApiError::NotFound),
            Err(e) => Err(e.into()),
        }
    }

    pub fn link_oidc(
        &self,
        profile_ulid: &str,
        provider: &str,
        sub: &str,
        display_label: Option<&str>,
        now_iso: &str,
    ) -> ApiResult<OidcLink> {
        let conn = self.pool.get()?;
        // INSERT OR IGNORE — second link of the same (provider, sub) keeps
        // the original profile_ulid, never moves the binding.
        conn.execute(
            "INSERT OR IGNORE INTO oidc_identities (profile_ulid, provider, sub, display_label, linked_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![profile_ulid, provider, sub, display_label, now_iso],
        )?;
        // Read back so we hand the caller the canonical row (in case the
        // ignore branch hit — bound to a different profile).
        let row = conn.query_row(
            "SELECT profile_ulid, display_label, linked_at FROM oidc_identities
             WHERE provider = ?1 AND sub = ?2",
            params![provider, sub],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?;
        if row.0 != profile_ulid {
            return Err(ApiError::Conflict);
        }
        Ok(OidcLink {
            provider: provider.to_string(),
            sub: sub.to_string(),
            display_label: row.1,
            linked_at: row.2,
        })
    }

    pub fn unlink_oidc(&self, profile_ulid: &str, provider: &str, sub: &str) -> ApiResult<()> {
        let conn = self.pool.get()?;
        let n = conn.execute(
            "DELETE FROM oidc_identities WHERE profile_ulid = ?1 AND provider = ?2 AND sub = ?3",
            params![profile_ulid, provider, sub],
        )?;
        if n == 0 {
            return Err(ApiError::NotFound);
        }
        Ok(())
    }

    pub fn list_oidc(&self, profile_ulid: &str) -> ApiResult<Vec<OidcLink>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT provider, sub, display_label, linked_at FROM oidc_identities
             WHERE profile_ulid = ?1 ORDER BY linked_at",
        )?;
        let rows = stmt
            .query_map(params![profile_ulid], |row| {
                Ok(OidcLink {
                    provider: row.get(0)?,
                    sub: row.get(1)?,
                    display_label: row.get(2)?,
                    linked_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_plan(&self, profile_ulid: &str, plan: Plan) -> ApiResult<()> {
        let conn = self.pool.get()?;
        let n = conn.execute(
            "UPDATE profiles SET plan = ?1 WHERE profile_ulid = ?2",
            params![plan.as_db_str(), profile_ulid],
        )?;
        if n == 0 {
            return Err(ApiError::NotFound);
        }
        Ok(())
    }

    pub fn list_payments(&self, profile_ulid: &str) -> ApiResult<Vec<Payment>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT ulid, created_at, amount_minor_units, currency, provider,
                    provider_charge_id, status, invoice_url
             FROM payment_records WHERE profile_ulid = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![profile_ulid], |row| {
                Ok(Payment {
                    ulid: row.get(0)?,
                    created_at: row.get(1)?,
                    amount_minor_units: row.get(2)?,
                    currency: row.get(3)?,
                    provider: row.get(4)?,
                    provider_charge_id: row.get(5)?,
                    status: row.get(6)?,
                    invoice_url: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

fn hash_recovery(code: &str) -> String {
    // Canonicalise before hashing — uppercase, strip whitespace + hyphens so
    // "ABCD EFGH\nIJKL MNOP", "abcd-efgh-ijkl-mnop" and "abcdefghijklmnop"
    // all hash identically. Crockford base32 has no hyphens or whitespace
    // in its native alphabet so this can't collide with real characters.
    let canonical: String = code
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_register_and_lookup() {
        let db = Db::in_memory().unwrap();
        let p = db
            .register_profile("01HMOCKULID1", "ABCD EFGH IJKL MNOP", "2026-05-12T00:00:00Z")
            .unwrap();
        assert_eq!(p.profile_ulid, "01HMOCKULID1");
        let found = db.lookup_by_recovery("abcd-efgh-ijkl-mnop").unwrap();
        assert_eq!(found.profile_ulid, "01HMOCKULID1");
    }

    #[test]
    fn conflicting_recovery_for_existing_ulid_errors() {
        let db = Db::in_memory().unwrap();
        db.register_profile("01HMOCKULID2", "AAAA", "2026-05-12T00:00:00Z").unwrap();
        let err = db
            .register_profile("01HMOCKULID2", "BBBB", "2026-05-12T00:00:00Z")
            .unwrap_err();
        matches!(err, ApiError::Conflict);
    }

    #[test]
    fn link_and_lookup_oidc() {
        let db = Db::in_memory().unwrap();
        db.register_profile("01HMOCKULID3", "ZZZZ", "2026-05-12T00:00:00Z").unwrap();
        db.link_oidc(
            "01HMOCKULID3",
            "https://accounts.google.com",
            "1234",
            Some("jakub@example.com"),
            "2026-05-12T00:00:00Z",
        )
        .unwrap();
        let p = db.lookup_by_oidc("https://accounts.google.com", "1234").unwrap();
        assert_eq!(p.profile_ulid, "01HMOCKULID3");
    }
}
