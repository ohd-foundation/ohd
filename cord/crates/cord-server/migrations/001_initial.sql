-- CORD server store. Holds CORD users (one per OIDC identity), the
-- data-source registry (encrypted share credentials), bring-your-own
-- model keys (encrypted), and chat history. No health data lands here.

CREATE TABLE IF NOT EXISTS users (
  cord_user_ulid TEXT PRIMARY KEY,
  oidc_issuer    TEXT NOT NULL,
  oidc_subject   TEXT NOT NULL,
  display_label  TEXT,
  created_at     TEXT NOT NULL,
  last_seen_at   TEXT NOT NULL,
  UNIQUE (oidc_issuer, oidc_subject)
);

-- A connected data source = one share credential. `enc_token` and the
-- pin are everything CORD needs to reach the user's storage; both are
-- sealed with the deployment data key (see crypto.rs).
CREATE TABLE IF NOT EXISTS data_sources (
  id              TEXT PRIMARY KEY,
  cord_user_ulid  TEXT NOT NULL REFERENCES users(cord_user_ulid),
  label           TEXT NOT NULL,
  kind            TEXT NOT NULL,            -- 'direct' | 'relay'
  endpoint        TEXT NOT NULL,            -- direct storage URL, or relay rendezvous URL
  rendezvous_id   TEXT,                     -- relay sources only
  relay_host      TEXT,                     -- relay sources only
  enc_token       TEXT NOT NULL,            -- sealed grant token (ohdg_...)
  cert_pin        TEXT,                     -- base64url sha256(SPKI); null = CA-cert direct
  scope_json      TEXT,                     -- cached scope summary from the share
  status          TEXT NOT NULL DEFAULT 'connected',
  created_at      TEXT NOT NULL,
  last_ok_at      TEXT
);
CREATE INDEX IF NOT EXISTS idx_sources_user ON data_sources(cord_user_ulid);

-- Bring-your-own model keys. Only populated when the deployment's
-- `[models.byo] allow_user_keys` is true.
CREATE TABLE IF NOT EXISTS byo_keys (
  id              TEXT PRIMARY KEY,
  cord_user_ulid  TEXT NOT NULL REFERENCES users(cord_user_ulid),
  provider_kind   TEXT NOT NULL,            -- 'anthropic' | 'gemini' | 'openai'
  label           TEXT NOT NULL,
  enc_api_key     TEXT NOT NULL,            -- sealed
  created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_byo_user ON byo_keys(cord_user_ulid);

CREATE TABLE IF NOT EXISTS chats (
  id              TEXT PRIMARY KEY,
  cord_user_ulid  TEXT NOT NULL REFERENCES users(cord_user_ulid),
  source_id       TEXT NOT NULL REFERENCES data_sources(id),
  model           TEXT NOT NULL,
  title           TEXT,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chats_user ON chats(cord_user_ulid);

CREATE TABLE IF NOT EXISTS chat_messages (
  id          TEXT PRIMARY KEY,
  chat_id     TEXT NOT NULL REFERENCES chats(id),
  role        TEXT NOT NULL,                -- 'user' | 'assistant'
  content     TEXT NOT NULL,
  created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_msgs_chat ON chat_messages(chat_id);
