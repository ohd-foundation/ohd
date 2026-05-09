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
        } else if let Some(rest) = ns.strip_prefix("com") {
            // form: com.<owner>.<name>
            // The first split gave us ns="com" and name="<owner>.<rest>".
            // Re-parse so the owner stays in the namespace.
            let _ = rest;
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
