-- OHD SaaS schema — email/password credentials.
--
-- Added for OHD Identity (`ohd-idp`): the IdP's first-party email/password
-- auth path. Passwords are held only as argon2id PHC strings — never
-- plaintext, never reversible. Each credential row points at a `profiles`
-- row; the `profile_ulid` is the stable OHD identity an `id_token` carries.
--
-- `ohd-idp` also runs this `CREATE TABLE IF NOT EXISTS` itself on startup,
-- so it works whether or not the SaaS migration runner has been here first.

CREATE TABLE IF NOT EXISTS email_credentials (
    email          TEXT PRIMARY KEY,      -- normalized lowercase
    profile_ulid   TEXT NOT NULL,
    password_hash  TEXT NOT NULL,         -- argon2id PHC string
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    FOREIGN KEY (profile_ulid) REFERENCES profiles(profile_ulid) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS email_credentials_profile_idx ON email_credentials(profile_ulid);
