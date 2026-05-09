-- OHD Storage: family / delegate access (grant kind = 'delegate').
--
-- Per `spec/storage-format.md` / `spec/components/storage.md` "Open design
-- items" — a `delegate` grant lets one user act on behalf of another (parent
-- on behalf of child, caregiver on behalf of elderly parent, etc.). The
-- delegate sees the underlying user's data; audit captures both identities
-- so the user can later see "my caregiver fetched X on date Y".
--
-- Authority shape (decided in this revision): **scoped**. The delegate sees
-- only the scope the grant defines (same per-event-type / per-channel /
-- per-sensitivity rules as a regular grant), not unrestricted access. The
-- user can also flag specific channels / event types as "self-only" via
-- normal `grant_*_rules` so even a delegate cannot read those.
--
-- Schema additions:
--   - `grants.delegate_for_user_ulid` — when set, this grant is a delegate
--     grant; the bearer (the delegate) reads as if querying that user's
--     data. The `grants.grantee_ulid` is the *delegate*'s identity.
--
-- Token resolution (see crates/ohd-storage-core/src/auth.rs): when a grant
-- token resolves to a row with non-NULL `delegate_for_user_ulid`, the
-- ResolvedToken's `effective_user_ulid` is set to that ULID, and audit
-- writes a second row with `actor_type='delegate'` and
-- `delegated_for_user_ulid` populated.
--
-- Idempotent migration: ALTER TABLE only runs the column add when the
-- column doesn't already exist.

-- SQLite doesn't have IF NOT EXISTS on ALTER TABLE; gate via PRAGMA.
-- The migration runner already gates by `_meta.mig:006_delegate_grants`,
-- so re-running this script is suppressed at the runner level. The
-- inner ALTER is unconditional within one application.

ALTER TABLE grants ADD COLUMN delegate_for_user_ulid BLOB;

CREATE INDEX IF NOT EXISTS idx_grants_delegate_for
  ON grants (delegate_for_user_ulid)
  WHERE delegate_for_user_ulid IS NOT NULL;

-- Audit log gets a sister column to capture the `effective_user_ulid` per
-- row when the actor is a delegate. NULL on every non-delegate row; set to
-- the user being acted on for delegate rows.
ALTER TABLE audit_log ADD COLUMN delegated_for_user_ulid BLOB;
