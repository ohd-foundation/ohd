-- OHD Storage: multi-identity account linking.
--
-- A single `user_ulid` may be associated with multiple OIDC `(provider, subject)`
-- identities. The user can sign in via any linked identity and resolve to the
-- same account; the identities are managed via `AuthService.{ListIdentities,
-- LinkIdentityStart, CompleteIdentityLink, UnlinkIdentity, SetPrimaryIdentity}`.
--
-- See `spec/auth.md` "Multiple identities per user" and STATUS.md
-- "Multi-identity account linking" for the design.
--
-- Real-world value:
--   - Account portability (move from Google to Facebook without losing data)
--   - Lost-account prevention (Google locks you out → sign in via Facebook)
--   - Operator/personal split (clinic SSO + personal Google on same OHD storage)
--
-- Schema additions:
--   - `oidc_identities` — created here (the previous design assumed a system DB,
--     but for v1 single-binary deployments we colocate this in the per-user file
--     under a `_` prefix, consistent with `_tokens`. Multi-tenant deployments
--     will lift this into the deployment-level system DB; see STATUS.md
--     "Decisions and deviations" point 2). Multiple rows per `user_ulid` are
--     allowed; uniqueness is on `(provider, subject)` only.
--   - `pending_identity_links` — temp state during the link OAuth flow; rows
--     auto-expire after 10 minutes via `expires_at_ms`.
--
-- Idempotent migration: CREATE TABLE IF NOT EXISTS guards every table.

CREATE TABLE IF NOT EXISTS _oidc_identities (
  id                    INTEGER PRIMARY KEY AUTOINCREMENT,
  user_ulid             BLOB NOT NULL,
  provider              TEXT NOT NULL,            -- OIDC issuer URL or short name
  subject               TEXT NOT NULL,            -- provider-issued opaque id (`sub` claim)
  email_hash            BLOB,                     -- sha256(email) for login-hint matching only
  display_label         TEXT,                     -- user-facing label, e.g. "Personal Google"
  is_primary            INTEGER NOT NULL DEFAULT 0,
  linked_at_ms          INTEGER NOT NULL,
  linked_via_actor_id   INTEGER,                  -- _tokens.id used during the link (audit)
  last_login_ms         INTEGER,
  UNIQUE (provider, subject)
);

CREATE INDEX IF NOT EXISTS idx_oidc_user        ON _oidc_identities (user_ulid);
CREATE INDEX IF NOT EXISTS idx_oidc_user_primary ON _oidc_identities (user_ulid)
  WHERE is_primary = 1;

CREATE TABLE IF NOT EXISTS _pending_identity_links (
  id                     INTEGER PRIMARY KEY AUTOINCREMENT,
  link_token             BLOB NOT NULL UNIQUE,    -- random 32-byte nonce
  requesting_user_ulid   BLOB NOT NULL,
  requesting_session_id  INTEGER,                 -- _tokens.id of the self-session that started the flow
  provider_hint          TEXT,                    -- e.g. 'google', 'facebook'
  created_at_ms          INTEGER NOT NULL,
  expires_at_ms          INTEGER NOT NULL,
  completed              INTEGER NOT NULL DEFAULT 0,
  completed_at_ms        INTEGER
);

CREATE INDEX IF NOT EXISTS idx_pending_links_user ON _pending_identity_links (requesting_user_ulid);
CREATE INDEX IF NOT EXISTS idx_pending_links_expires ON _pending_identity_links (expires_at_ms)
  WHERE completed = 0;
