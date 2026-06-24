//! `ShareScope` — grant-scope enforcement for the share responder.
//!
//! When OHD Connect serves a remote consumer (CORD, a clinician's device)
//! over the relay tunnel it must NOT run the tool loop as the storage
//! owner. The consumer reaches the phone through a **share** whose grant
//! constrains what may be read or written. This module materializes that
//! grant into a [`ShareScope`] and threads it through dispatch.
//!
//! See `cord/spec/data-link.md` "The phone-side share responder" →
//! "Scope enforcement". The phone is the enforcement boundary: a buggy or
//! compromised CORD cannot exceed what the user granted.
//!
//! `None` everywhere (the owner / local-CORD path) leaves behaviour
//! unchanged — the scope is purely additive.

use ohd_storage_core::grants::{GrantRow, RuleEffect};
use serde_json::Value;

/// How a tool relates to a share scope. Drives both catalog filtering and
/// dispatch gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// Stateless utility (`now`) — always available, touches no data.
    Utility,
    /// Reads the user's events. Subject to event-type / channel / time
    /// intersection.
    Read,
    /// Writes a new event. Available only when the grant carries write
    /// rules.
    Write,
    /// Grant / case / audit administration. Owner-only — never exposed to
    /// a share, regardless of grant rules.
    Operator,
}

/// Classify a tool by name. Unknown names are treated as `Operator` (the
/// most restrictive category) so a newly-added tool is never silently
/// exposed to a share before it has been categorized here.
pub fn tool_kind(name: &str) -> ToolKind {
    match name {
        "now" => ToolKind::Utility,
        "query_events" | "query_latest" | "describe_data" | "summarize" | "correlate"
        | "chart" | "get_food_log" | "get_medications_taken"
        | "list_allergies" | "list_conditions" | "list_emergency_contacts"
        | "get_health_profile" | "list_active_regimens" => ToolKind::Read,
        "log_symptom" | "log_food" | "log_medication" | "log_measurement" | "log_exercise"
        | "log_mood" | "log_sleep" | "log_free_event"
        | "record_allergy" | "remove_allergy" | "record_condition" | "resolve_condition"
        | "set_blood_type" | "record_emergency_contact" | "remove_emergency_contact"
        | "start_medication_regimen" | "discontinue_medication_regimen" => ToolKind::Write,
        _ => ToolKind::Operator,
    }
}

/// Materialized read/write rules + time window for a single share.
///
/// Derived from a [`GrantRow`] via [`ShareScope::from_grant`]. Operates at
/// the dotted-event-type / channel-path granularity the MCP tools use (the
/// numeric-id [`ohd_storage_core::events::GrantScope`] is the OHDC-layer
/// counterpart; this is the tool-layer one).
#[derive(Debug, Clone)]
pub struct ShareScope {
    /// Action for an event type matched by no explicit read rule.
    default_read_allow: bool,
    /// Dotted event types with an `allow` read rule.
    read_allow: Vec<String>,
    /// Dotted event types with a `deny` read rule. Deny wins.
    read_deny: Vec<String>,
    /// Per-`(event_type, channel_path)` channel read rules.
    channel_rules: Vec<ChannelRule>,
    /// Dotted event types the grant permits writing. Empty = read-only
    /// scope (every write tool is hidden + rejected).
    write_allow: Vec<String>,
    /// Rolling-window bound in days, if any.
    rolling_window_days: Option<i32>,
    /// Absolute `[from_ms, to_ms]` window, if any.
    absolute_window: Option<(i64, i64)>,
    /// `now` captured at construction so a slow session sees a stable
    /// rolling cutoff.
    now_ms: i64,
    /// When set, the scope denies everything: suspended, revoked, or
    /// expired grant. Carries a short human-readable reason.
    denied: Option<String>,
}

#[derive(Debug, Clone)]
struct ChannelRule {
    event_type: String,
    channel_path: String,
    allow: bool,
}

/// The lower / upper time bounds a query is permitted to span, after
/// intersecting a requested window with the scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowBounds {
    /// Inclusive lower bound (Unix ms), or `None` for unbounded.
    pub from_ms: Option<i64>,
    /// Inclusive upper bound (Unix ms), or `None` for unbounded.
    pub to_ms: Option<i64>,
    /// `true` when the scope and the request do not overlap at all — the
    /// caller should return an empty result rather than query storage.
    pub empty: bool,
}

impl ShareScope {
    /// Build a scope from a grant row.
    ///
    /// A suspended (`suspended_at_ms`), revoked (`revoked_at_ms`), or
    /// expired (`expires_at_ms <= now`) grant yields a deny-all scope that
    /// keeps its rules but refuses every request — matching the storage
    /// rule that a suspended grant "resolves to deny all without losing
    /// its rules".
    pub fn from_grant(grant: &GrantRow, now_ms: i64) -> Self {
        let denied = if grant.suspended_at_ms.is_some() {
            Some("share is suspended".to_string())
        } else if grant.revoked_at_ms.is_some() {
            Some("share has been revoked".to_string())
        } else if grant.expires_at_ms.is_some_and(|e| e <= now_ms) {
            Some("share has expired".to_string())
        } else {
            None
        };

        let mut read_allow = Vec::new();
        let mut read_deny = Vec::new();
        for (et, effect) in &grant.event_type_rules {
            match effect {
                RuleEffect::Allow => read_allow.push(et.clone()),
                RuleEffect::Deny => read_deny.push(et.clone()),
            }
        }
        let channel_rules = grant
            .channel_rules
            .iter()
            .map(|c| ChannelRule {
                event_type: c.event_type.clone(),
                channel_path: c.channel_path.clone(),
                allow: matches!(c.effect, RuleEffect::Allow),
            })
            .collect();
        let write_allow = grant
            .write_event_type_rules
            .iter()
            .filter(|(_, e)| matches!(e, RuleEffect::Allow))
            .map(|(et, _)| et.clone())
            .collect();

        let rolling_window_days = grant.rolling_window_days;
        let absolute_window = grant.absolute_window;

        ShareScope {
            default_read_allow: grant.default_action == "allow",
            read_allow,
            read_deny,
            channel_rules,
            write_allow,
            rolling_window_days,
            absolute_window,
            now_ms,
            denied,
        }
    }

    /// `true` when the scope denies everything (suspended / revoked /
    /// expired grant).
    pub fn is_denied(&self) -> bool {
        self.denied.is_some()
    }

    /// Human-readable deny reason, when [`Self::is_denied`].
    pub fn deny_reason(&self) -> Option<&str> {
        self.denied.as_deref()
    }

    /// `true` when the grant carries at least one `allow` write rule.
    pub fn allows_any_write(&self) -> bool {
        !self.denied.is_some() && !self.write_allow.is_empty()
    }

    /// Whether a tool of the given kind is visible / callable under this
    /// scope. Operator tools are owner-only; write tools require write
    /// rules; a denied scope still lists read tools but every call to
    /// them returns "not permitted" (so the consumer sees a stable
    /// catalog rather than an empty one mid-session).
    pub fn allows_tool_kind(&self, kind: ToolKind) -> bool {
        match kind {
            ToolKind::Utility | ToolKind::Read => true,
            ToolKind::Write => self.allows_any_write(),
            ToolKind::Operator => false,
        }
    }

    /// Whether reads of `event_type` are permitted, by the read-rule
    /// precedence ladder (deny > allow > default). A denied scope refuses
    /// everything.
    pub fn allows_read_type(&self, event_type: &str) -> bool {
        if self.denied.is_some() {
            return false;
        }
        if self.read_deny.iter().any(|t| t == event_type) {
            return false;
        }
        if self.read_allow.iter().any(|t| t == event_type) {
            return true;
        }
        self.default_read_allow
    }

    /// Whether writes of `event_type` are permitted.
    pub fn allows_write_type(&self, event_type: &str) -> bool {
        if self.denied.is_some() {
            return false;
        }
        self.write_allow.iter().any(|t| t == event_type)
    }

    /// Whether a single channel of an in-scope event survives redaction.
    ///
    /// Channel rules are the finer grain: an explicit channel rule wins
    /// over the event-type decision; with no channel rule the channel
    /// inherits its event type's read decision.
    pub fn allows_channel(&self, event_type: &str, channel_path: &str) -> bool {
        if self.denied.is_some() {
            return false;
        }
        if let Some(rule) = self
            .channel_rules
            .iter()
            .find(|r| r.event_type == event_type && r.channel_path == channel_path)
        {
            return rule.allow;
        }
        self.allows_read_type(event_type)
    }

    /// Intersect a requested `[from_ms, to_ms]` window with the scope's
    /// time window. The returned bounds are what a read tool must clamp
    /// to; `empty == true` means no overlap (return an empty result).
    pub fn clamp_window(
        &self,
        requested_from: Option<i64>,
        requested_to: Option<i64>,
    ) -> WindowBounds {
        if self.denied.is_some() {
            return WindowBounds { from_ms: requested_from, to_ms: requested_to, empty: true };
        }
        // Lower bound: the scope's earliest visible instant.
        let mut scope_from: Option<i64> = None;
        if let Some(days) = self.rolling_window_days {
            let cutoff = self.now_ms.saturating_sub(days as i64 * 86_400_000);
            scope_from = Some(cutoff);
        }
        if let Some((from, _)) = self.absolute_window {
            scope_from = Some(scope_from.map_or(from, |s| s.max(from)));
        }
        let scope_to: Option<i64> = self.absolute_window.map(|(_, to)| to);

        let from = max_opt(requested_from, scope_from);
        let to = min_opt(requested_to, scope_to);
        let empty = match (from, to) {
            (Some(f), Some(t)) => f > t,
            _ => false,
        };
        WindowBounds { from_ms: from, to_ms: to, empty }
    }

    /// Post-process a tool result: drop event rows whose `event_type` is
    /// out of scope, redact out-of-scope channels from the rest, and drop
    /// any row falling outside the scope's time window. Walks any
    /// `events` array a read tool emits. A no-op when nothing matches —
    /// safe to call on every read result.
    pub fn redact_result(&self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                if let Some(Value::Array(events)) = map.get_mut("events") {
                    self.redact_event_array(events);
                    let count = events.len();
                    if let Some(c) = map.get_mut("count") {
                        *c = Value::from(count);
                    }
                }
                // Recurse into nested objects (e.g. correlate's `pairs`).
                for (_k, v) in map.iter_mut() {
                    if v.is_array() || v.is_object() {
                        self.redact_nested(v);
                    }
                }
            }
            Value::Array(items) => {
                for item in items.iter_mut() {
                    self.redact_result(item);
                }
            }
            _ => {}
        }
    }

    fn redact_nested(&self, value: &mut Value) {
        if let Value::Array(items) = value {
            // Heuristic: an array of objects each carrying `event_type`
            // is an event-like array; redact it.
            if items.iter().any(|i| i.get("event_type").is_some()) {
                self.redact_event_array(items);
            } else {
                for item in items.iter_mut() {
                    self.redact_nested(item);
                }
            }
        } else if let Value::Object(map) = value {
            for (_k, v) in map.iter_mut() {
                self.redact_nested(v);
            }
        }
    }

    fn redact_event_array(&self, events: &mut Vec<Value>) {
        events.retain_mut(|ev| {
            let Some(et) = ev.get("event_type").and_then(|v| v.as_str()).map(String::from)
            else {
                // No event_type — leave the row untouched (not an event).
                return true;
            };
            if !self.allows_read_type(&et) {
                return false;
            }
            if let Some(ts) = ev.get("ts_ms").and_then(|v| v.as_i64()) {
                let w = self.clamp_window(None, None);
                if w.empty
                    || w.from_ms.is_some_and(|f| ts < f)
                    || w.to_ms.is_some_and(|t| ts > t)
                {
                    return false;
                }
            }
            if let Some(Value::Object(channels)) = ev.get_mut("channels") {
                channels.retain(|path, _| self.allows_channel(&et, path));
            }
            true
        });
    }
}

fn max_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
}

fn min_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohd_storage_core::grants::ChannelRuleSpec;
    use serde_json::json;

    const NOW: i64 = 1_700_000_000_000;
    const DAY: i64 = 86_400_000;

    /// A minimal grant row — every field zeroed / empty, ready to tweak.
    fn bare_grant() -> GrantRow {
        GrantRow {
            id: 1,
            ulid: [0u8; 16],
            grantee_label: "CORD".into(),
            grantee_kind: "agent".into(),
            grantee_ulid: None,
            delegate_for_user_ulid: None,
            purpose: None,
            created_at_ms: NOW - 10 * DAY,
            expires_at_ms: None,
            revoked_at_ms: None,
            suspended_at_ms: None,
            default_action: "deny".into(),
            aggregation_only: false,
            strip_notes: false,
            require_approval_per_query: false,
            approval_mode: "never_required".into(),
            notify_on_access: false,
            max_queries_per_day: None,
            max_queries_per_hour: None,
            rolling_window_days: None,
            absolute_window: None,
            event_type_rules: vec![],
            channel_rules: vec![],
            sensitivity_rules: vec![],
            write_event_type_rules: vec![],
            auto_approve_event_types: vec![],
            class_key_wraps: Default::default(),
            grantee_recovery_pubkey: None,
            issuer_recovery_pubkey: None,
        }
    }

    #[test]
    fn in_scope_type_is_allowed_out_of_scope_denied() {
        let mut g = bare_grant();
        g.event_type_rules = vec![("measurement.glucose".into(), RuleEffect::Allow)];
        let scope = ShareScope::from_grant(&g, NOW);
        assert!(scope.allows_read_type("measurement.glucose"));
        // default_action = deny → an unlisted type is denied.
        assert!(!scope.allows_read_type("symptom.headache"));
    }

    #[test]
    fn channel_redaction_drops_out_of_scope_channels() {
        let mut g = bare_grant();
        g.default_action = "allow".into();
        g.channel_rules = vec![ChannelRuleSpec {
            event_type: "measurement.bp".into(),
            channel_path: "diastolic_mmhg".into(),
            effect: RuleEffect::Deny,
        }];
        let scope = ShareScope::from_grant(&g, NOW);
        assert!(scope.allows_channel("measurement.bp", "systolic_mmhg"));
        assert!(!scope.allows_channel("measurement.bp", "diastolic_mmhg"));

        let mut result = json!({
            "count": 1,
            "events": [{
                "event_type": "measurement.bp",
                "ts_ms": NOW - DAY,
                "channels": { "systolic_mmhg": 120, "diastolic_mmhg": 80 }
            }]
        });
        scope.redact_result(&mut result);
        let chans = result["events"][0]["channels"].as_object().unwrap();
        assert!(chans.contains_key("systolic_mmhg"));
        assert!(!chans.contains_key("diastolic_mmhg"), "out-of-scope channel redacted");
    }

    #[test]
    fn out_of_scope_event_rows_are_dropped() {
        let mut g = bare_grant();
        g.event_type_rules = vec![("food.eaten".into(), RuleEffect::Allow)];
        let scope = ShareScope::from_grant(&g, NOW);
        let mut result = json!({
            "count": 2,
            "events": [
                { "event_type": "food.eaten", "ts_ms": NOW - DAY, "channels": {} },
                { "event_type": "symptom.headache", "ts_ms": NOW - DAY, "channels": {} }
            ]
        });
        scope.redact_result(&mut result);
        let events = result["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event_type"], "food.eaten");
        assert_eq!(result["count"], 1, "count is corrected after redaction");
    }

    #[test]
    fn write_tools_hidden_for_a_read_only_scope() {
        let g = bare_grant(); // no write_event_type_rules
        let scope = ShareScope::from_grant(&g, NOW);
        assert!(!scope.allows_any_write());
        assert!(!scope.allows_tool_kind(ToolKind::Write));
        assert!(scope.allows_tool_kind(ToolKind::Read));
        // Operator tools are owner-only even for a fully-privileged grant.
        assert!(!scope.allows_tool_kind(ToolKind::Operator));
    }

    #[test]
    fn write_scope_permits_only_its_write_types() {
        let mut g = bare_grant();
        g.write_event_type_rules = vec![("food.eaten".into(), RuleEffect::Allow)];
        let scope = ShareScope::from_grant(&g, NOW);
        assert!(scope.allows_any_write());
        assert!(scope.allows_tool_kind(ToolKind::Write));
        assert!(scope.allows_write_type("food.eaten"));
        assert!(!scope.allows_write_type("medication.taken"));
    }

    #[test]
    fn suspended_scope_denies_everything() {
        let mut g = bare_grant();
        g.default_action = "allow".into();
        g.write_event_type_rules = vec![("food.eaten".into(), RuleEffect::Allow)];
        g.suspended_at_ms = Some(NOW - DAY);
        let scope = ShareScope::from_grant(&g, NOW);
        assert!(scope.is_denied());
        assert!(!scope.allows_read_type("food.eaten"));
        assert!(!scope.allows_write_type("food.eaten"));
        assert!(!scope.allows_any_write());
        assert!(scope.clamp_window(None, None).empty);
    }

    #[test]
    fn expired_and_revoked_scopes_deny_everything() {
        let mut expired = bare_grant();
        expired.default_action = "allow".into();
        expired.expires_at_ms = Some(NOW - DAY);
        assert!(ShareScope::from_grant(&expired, NOW).is_denied());

        let mut revoked = bare_grant();
        revoked.default_action = "allow".into();
        revoked.revoked_at_ms = Some(NOW - DAY);
        assert!(ShareScope::from_grant(&revoked, NOW).is_denied());
    }

    #[test]
    fn rolling_window_clamps_the_requested_lower_bound() {
        let mut g = bare_grant();
        g.default_action = "allow".into();
        g.rolling_window_days = Some(7);
        let scope = ShareScope::from_grant(&g, NOW);
        // Request 30 days; scope only allows 7 → lower bound moves up.
        let w = scope.clamp_window(Some(NOW - 30 * DAY), Some(NOW));
        assert_eq!(w.from_ms, Some(NOW - 7 * DAY));
        assert_eq!(w.to_ms, Some(NOW));
        assert!(!w.empty);
    }

    #[test]
    fn absolute_window_with_no_overlap_is_empty() {
        let mut g = bare_grant();
        g.default_action = "allow".into();
        g.absolute_window = Some((NOW - 100 * DAY, NOW - 90 * DAY));
        let scope = ShareScope::from_grant(&g, NOW);
        // Request the last 10 days — entirely after the grant's window.
        let w = scope.clamp_window(Some(NOW - 10 * DAY), Some(NOW));
        assert!(w.empty, "disjoint windows must be flagged empty");
    }
}
