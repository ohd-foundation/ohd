//! Channel and event-type registry — lookup + alias resolution.
//!
//! Backs the `event_types`, `channels`, `type_aliases`, `channel_aliases`
//! tables (see `spec/storage-format.md` "SQL schema" and "Channel registry").
//! Validates writes against the registry; resolves aliases at read time.

use rusqlite::{params, Connection, OptionalExtension};

use crate::{Error, Result};

/// Logical identifier of an event type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventTypeName {
    /// Namespace, e.g. `"std"` or `"com.acme"`.
    pub namespace: String,
    /// Local name within the namespace.
    pub name: String,
}

impl EventTypeName {
    /// Parse a dotted form like `"std.glucose"` or `"com.acme.thing"`.
    pub fn parse(s: &str) -> Result<Self> {
        let (ns, name) = match s.split_once('.') {
            Some(p) => p,
            None => {
                return Err(Error::InvalidArgument(format!(
                    "event_type missing namespace: {s}"
                )))
            }
        };
        if ns == "std" {
            Ok(EventTypeName {
                namespace: ns.to_string(),
                name: name.to_string(),
            })
        } else if ns == "com" {
            // form: com.<owner>.<name>
            // The first split gave us ns="com" and name="<owner>.<rest>".
            // Re-parse so the owner stays in the namespace.
            //
            // (Earlier this checked `ns.strip_prefix("com")`, which silently
            // misclassified `composition.X.Y` / `command.X.Y` / any other
            // `com…`-prefixed namespace as `com.<owner>.<leaf>`. Exact
            // match is correct.)
            let (owner, leaf) = match name.split_once('.') {
                Some(p) => p,
                None => {
                    return Err(Error::InvalidArgument(format!(
                        "com.* event_type missing owner: {s}"
                    )))
                }
            };
            Ok(EventTypeName {
                namespace: format!("com.{owner}"),
                name: leaf.to_string(),
            })
        } else {
            Ok(EventTypeName {
                namespace: ns.to_string(),
                name: name.to_string(),
            })
        }
    }

    /// Render back to dotted form.
    pub fn as_dotted(&self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }
}

/// Value type of a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    /// 64-bit float.
    Real,
    /// 64-bit signed integer.
    Int,
    /// Boolean (stored in `value_int` 0/1).
    Bool,
    /// Free text.
    Text,
    /// Append-only ordinal.
    Enum,
    /// Group node — carries no value.
    Group,
}

impl ValueType {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "real" => ValueType::Real,
            "int" => ValueType::Int,
            "bool" => ValueType::Bool,
            "text" => ValueType::Text,
            "enum" => ValueType::Enum,
            "group" => ValueType::Group,
            other => {
                return Err(Error::InvalidArgument(format!(
                    "unknown value_type: {other}"
                )))
            }
        })
    }
}

/// Resolved row for an event type.
#[derive(Debug, Clone)]
pub struct EventTypeRow {
    /// `event_types.id`.
    pub id: i64,
    /// Namespace.
    pub namespace: String,
    /// Local name.
    pub name: String,
    /// Sensitivity class default for this type.
    pub default_sensitivity_class: String,
}

/// Resolved row for a channel.
#[derive(Debug, Clone)]
pub struct ChannelRow {
    /// `channels.id`.
    pub id: i64,
    /// Event type this channel lives on.
    pub event_type_id: i64,
    /// Dot-separated path.
    pub path: String,
    /// Value type.
    pub value_type: ValueType,
    /// Canonical unit.
    pub unit: Option<String>,
    /// Append-only enum values; empty for non-enum channels.
    pub enum_values: Vec<String>,
    /// Sensitivity class.
    pub sensitivity_class: String,
}

/// Resolve a wire event-type name (possibly an alias) to a registry row.
///
/// Returns [`Error::UnknownType`] if not found.
pub fn resolve_event_type(conn: &Connection, name: &EventTypeName) -> Result<EventTypeRow> {
    let direct: Option<EventTypeRow> = conn
        .query_row(
            "SELECT id, namespace, name, default_sensitivity_class
               FROM event_types WHERE namespace = ?1 AND name = ?2",
            params![name.namespace, name.name],
            |r| {
                Ok(EventTypeRow {
                    id: r.get(0)?,
                    namespace: r.get(1)?,
                    name: r.get(2)?,
                    default_sensitivity_class: r.get(3)?,
                })
            },
        )
        .optional()?;
    if let Some(row) = direct {
        return Ok(row);
    }
    // Alias path: type_aliases (old_namespace, old_name) → new_event_type_id
    let aliased: Option<i64> = conn
        .query_row(
            "SELECT new_event_type_id FROM type_aliases
              WHERE old_namespace = ?1 AND old_name = ?2",
            params![name.namespace, name.name],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(id) = aliased {
        return event_type_by_id(conn, id);
    }
    Err(Error::UnknownType(name.as_dotted()))
}

/// Resolve a wire event-type name, transparently falling back to its
/// `custom.<original_dotted>` shadow when the canonical row is missing.
///
/// Use this on both reads and writes when you want open-ended-schema semantics:
/// users can keep writing `composition.allergen.gluten` even before that type
/// is promoted to the canonical registry — the data lands under
/// `custom.composition.allergen.gluten` (registered on first write) and is
/// transparently found by both read variants of this lookup.
///
/// Already-`custom.*` names skip the fallback (no `custom.custom.foo`).
pub fn resolve_event_type_with_custom_fallback(
    conn: &Connection,
    name: &EventTypeName,
) -> Result<EventTypeRow> {
    match resolve_event_type(conn, name) {
        Ok(row) => Ok(row),
        Err(Error::UnknownType(_)) if name.namespace != "custom" => {
            let shadow = EventTypeName {
                namespace: "custom".to_string(),
                name: name.as_dotted(),
            };
            resolve_event_type(conn, &shadow)
        }
        Err(e) => Err(e),
    }
}

/// Auto-register an unknown event type under the `custom.*` namespace.
///
/// Used by `put_events` when a writer submits a name that's not in the
/// canonical registry. The original name is preserved as the suffix so a
/// future migration can promote it via a single `UPDATE event_types` (or by
/// inserting the canonical row and a `type_aliases` row pointing the custom
/// shadow at it).
///
/// Idempotent on the (namespace, name) UNIQUE: a second call returns the
/// row inserted by the first.
pub fn register_event_type_as_custom(
    conn: &Connection,
    original: &EventTypeName,
) -> Result<EventTypeRow> {
    // Don't double-prefix custom.X → custom.custom.X.
    let custom_name = if original.namespace == "custom" {
        original.name.clone()
    } else {
        original.as_dotted()
    };
    let description = format!("Auto-registered (was: {})", original.as_dotted());
    conn.execute(
        "INSERT OR IGNORE INTO event_types
             (namespace, name, description, default_sensitivity_class)
         VALUES ('custom', ?1, ?2, 'general')",
        params![custom_name, description],
    )?;
    resolve_event_type(
        conn,
        &EventTypeName {
            namespace: "custom".to_string(),
            name: custom_name,
        },
    )
}

/// Look up event type by primary key.
pub fn event_type_by_id(conn: &Connection, id: i64) -> Result<EventTypeRow> {
    conn.query_row(
        "SELECT id, namespace, name, default_sensitivity_class
           FROM event_types WHERE id = ?1",
        params![id],
        |r| {
            Ok(EventTypeRow {
                id: r.get(0)?,
                namespace: r.get(1)?,
                name: r.get(2)?,
                default_sensitivity_class: r.get(3)?,
            })
        },
    )
    .map_err(|_| Error::UnknownType(format!("id={id}")))
}

/// Resolve a `(event_type_id, channel_path)` pair to a channel row, including
/// channel_aliases.
pub fn resolve_channel(
    conn: &Connection,
    event_type_id: i64,
    channel_path: &str,
) -> Result<ChannelRow> {
    let direct: Option<ChannelRow> = conn
        .query_row(
            "SELECT id, event_type_id, path, value_type, unit, enum_values, sensitivity_class
               FROM channels WHERE event_type_id = ?1 AND path = ?2",
            params![event_type_id, channel_path],
            channel_row_from_row,
        )
        .optional()?;
    if let Some(row) = direct {
        return Ok(row);
    }
    let aliased: Option<i64> = conn
        .query_row(
            "SELECT new_channel_id FROM channel_aliases
              WHERE event_type_id = ?1 AND old_path = ?2",
            params![event_type_id, channel_path],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(cid) = aliased {
        return channel_by_id(conn, cid);
    }
    let event_type = event_type_by_id(conn, event_type_id)?;
    Err(Error::UnknownChannel {
        event_type: format!("{}.{}", event_type.namespace, event_type.name),
        channel_path: channel_path.to_string(),
    })
}

/// Resolve a channel, **auto-registering it on first use** if no row exists.
///
/// Connect-side event types (food, measurement, symptom, …) are open-shape:
/// the producer knows the channels its UI emits, but pre-listing every
/// nutriment / micronutrient / OFF tag in `migrations/*.sql` is brittle —
/// adding a field to the UI required a Rust-core rebuild before this
/// helper existed. Instead we let `put_events` register unknown channels
/// inline, inferring the [`ValueType`] from the supplied scalar variant.
///
/// Inferred fields:
///  - `value_type`            ← scalar variant
///  - `unit`                  ← `NULL` (the path-name carries unit suffixes
///                              like `_mg` / `_g` in practice; explicit
///                              unit metadata stays opt-in via migrations)
///  - `sensitivity_class`     ← inherits the event type's default
///  - `parent_id`             ← `NULL`
///  - `enum_values`           ← `NULL`
///
/// Idempotent: a second call with the same `(event_type_id, channel_path)`
/// hits the existing row (via [`resolve_channel`]) and never re-inserts.
pub fn resolve_channel_or_register(
    conn: &Connection,
    event_type_id: i64,
    channel_path: &str,
    inferred_value_type: ValueType,
) -> Result<ChannelRow> {
    match resolve_channel(conn, event_type_id, channel_path) {
        Ok(row) => Ok(row),
        Err(Error::UnknownChannel { .. }) => {
            let etype = event_type_by_id(conn, event_type_id)?;
            let value_type_str = match inferred_value_type {
                ValueType::Real => "real",
                ValueType::Int => "int",
                ValueType::Bool => "bool",
                ValueType::Text => "text",
                ValueType::Enum => "enum",
                ValueType::Group => "group",
            };
            conn.execute(
                "INSERT OR IGNORE INTO channels
                     (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
                 VALUES (?1, NULL, ?2, ?2, ?3, NULL, ?4)",
                params![
                    event_type_id,
                    channel_path,
                    value_type_str,
                    etype.default_sensitivity_class,
                ],
            )?;
            resolve_channel(conn, event_type_id, channel_path)
        }
        Err(e) => Err(e),
    }
}

/// Look up channel by primary key.
pub fn channel_by_id(conn: &Connection, id: i64) -> Result<ChannelRow> {
    conn.query_row(
        "SELECT id, event_type_id, path, value_type, unit, enum_values, sensitivity_class
           FROM channels WHERE id = ?1",
        params![id],
        channel_row_from_row,
    )
    .map_err(|_| Error::InvalidArgument(format!("unknown channel id={id}")))
}

fn channel_row_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChannelRow> {
    let id: i64 = r.get(0)?;
    let event_type_id: i64 = r.get(1)?;
    let path: String = r.get(2)?;
    let vt: String = r.get(3)?;
    let unit: Option<String> = r.get(4)?;
    let enum_values_json: Option<String> = r.get(5)?;
    let sensitivity_class: String = r.get(6)?;
    let value_type = ValueType::parse(&vt).map_err(|_| {
        rusqlite::Error::InvalidColumnType(3, "value_type".to_string(), rusqlite::types::Type::Text)
    })?;
    let enum_values: Vec<String> = match enum_values_json {
        Some(s) => serde_json::from_str(&s).unwrap_or_default(),
        None => vec![],
    };
    Ok(ChannelRow {
        id,
        event_type_id,
        path,
        value_type,
        unit,
        enum_values,
        sensitivity_class,
    })
}
