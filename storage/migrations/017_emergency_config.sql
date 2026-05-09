-- OHD Storage: emergency / break-glass configuration (operator-side state).
--
-- Per `connect/spec/screens-emergency.md` "OHD Connect — patient side →
-- Settings tab: 'Emergency / Break-glass'". Stores the patient's
-- emergency-access preferences as a singleton row keyed by `user_ulid`.
--
-- The eight sections of the screens-emergency spec map onto the columns
-- below:
--
--   1. Feature toggle              → `enabled`
--   2. Discovery                   → `bluetooth_beacon`
--   3. Approval timing             → `approval_timeout_seconds`,
--                                    `default_action_on_timeout`
--   4. Lock-screen behaviour       → `lock_screen_visibility`
--   5. What responders see         → `history_window_hours`,
--                                    `channel_paths_allowed_json`,
--                                    `sensitivity_classes_allowed_json`
--   6. Location                    → `share_location`
--   7. Trusted authorities         → `trusted_authorities_json`
--   8. Advanced (bystander proxy)  → `bystander_proxy_enabled`
--
-- Schema is deliberately flat: lists land as JSON blobs because they're
-- read/written together (UI roundtrip, not queried). The singleton-per-user
-- shape mirrors `_notification_config` (migration 009).

CREATE TABLE IF NOT EXISTS _emergency_config (
    user_ulid                          BLOB PRIMARY KEY,             -- 16 B
    enabled                            INTEGER NOT NULL DEFAULT 0,   -- 0/1
    bluetooth_beacon                   INTEGER NOT NULL DEFAULT 1,
    approval_timeout_seconds           INTEGER NOT NULL DEFAULT 30,  -- 10..300
    default_action_on_timeout          TEXT    NOT NULL DEFAULT 'allow', -- 'allow' | 'refuse'
    lock_screen_visibility             TEXT    NOT NULL DEFAULT 'full',  -- 'full' | 'basic_only'
    history_window_hours               INTEGER NOT NULL DEFAULT 24,  -- 0|3|12|24
    channel_paths_allowed_json         TEXT    NOT NULL DEFAULT '[]',
    sensitivity_classes_allowed_json   TEXT    NOT NULL DEFAULT '[]',
    share_location                     INTEGER NOT NULL DEFAULT 0,
    trusted_authorities_json           TEXT    NOT NULL DEFAULT '[]',
    bystander_proxy_enabled            INTEGER NOT NULL DEFAULT 1,
    updated_at_ms                      INTEGER NOT NULL
);
