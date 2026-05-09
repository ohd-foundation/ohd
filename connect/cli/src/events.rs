//! `ohd-connect log <kind> ...` — event-input builders.
//!
//! Maps CLI args onto the channel keys defined in
//! `../../storage/migrations/002_std_registry.sql`. The mapping is hand-
//! written (one builder per kind) because the registry shape varies enough
//! that a generic value→channel resolver would obscure the intent. When
//! storage exposes `Registry.ResolveChannel` over OHDC (deferred per
//! `../STATUS.md`), this file shrinks to a thin proto adapter.
//!
//! Channel-key map (per the v1 std registry):
//!
//! | CLI subcommand              | OHDC `event_type`        | Channels written |
//! |-----------------------------|--------------------------|------------------|
//! | `log glucose <v> --unit u`  | `std.blood_glucose`      | `value` (real, mmol/L canonical) |
//! | `log heart_rate <v>`        | `std.heart_rate_resting` | `value` (real, bpm) |
//! | `log temperature <v> --unit u` | `std.body_temperature` | `value` (real, C canonical) |
//! | `log medication_taken <name> [--dose v --dose-unit u --status s]` | `std.medication_dose` | `name`, `dose`, `dose_unit` (enum), `status` (enum, default `taken`) |
//! | `log symptom <name> [--severity n --location s]` | `std.symptom` | `name`, `severity`, `location` |
//!
//! Unit conversion: glucose accepts `mg/dL` and converts to `mmol/L` (×1/18.0182)
//! before storage; temperature accepts `F` and converts to `C` before storage.
//! All other units are pass-through; storage's registry validates the canonical
//! unit per `events.rs` in the core.

use anyhow::{anyhow, Result};

use crate::proto::ohdc::v0 as pb;

/// Subcommand parameters captured from clap. Each variant shapes a single
/// `EventInput`.
#[derive(Debug, Clone)]
pub enum LogKind {
    Glucose {
        value: f64,
        unit: Option<String>,
    },
    HeartRate {
        bpm: f64,
    },
    Temperature {
        value: f64,
        unit: Option<String>,
    },
    MedicationTaken {
        name: String,
        dose: Option<f64>,
        dose_unit: Option<String>,
        status: Option<String>,
    },
    Symptom {
        name: String,
        severity: Option<i64>,
        location: Option<String>,
    },
}

/// Resolve the canonical OHDC type the storage registry stores under.
/// (Aliases are also resolved server-side, but using the canonical name keeps
/// query commands consistent — see `query_event_type_alias`.)
pub fn canonical_event_type(kind: &LogKind) -> &'static str {
    match kind {
        LogKind::Glucose { .. } => "std.blood_glucose",
        LogKind::HeartRate { .. } => "std.heart_rate_resting",
        LogKind::Temperature { .. } => "std.body_temperature",
        LogKind::MedicationTaken { .. } => "std.medication_dose",
        LogKind::Symptom { .. } => "std.symptom",
    }
}

/// User-facing aliases for the `ohd-connect query` short-form names.
/// `glucose` → `std.blood_glucose`, `heart_rate` → `std.heart_rate_resting`,
/// etc. Storage also resolves `std.glucose` server-side via `type_aliases`
/// (per `../../storage/migrations/002_std_registry.sql`), but the CLI
/// normalizes to the canonical name on the way in for predictable
/// round-trips.
///
/// Returns `None` for inputs that don't match a known short form. Callers
/// must short-circuit fully-qualified names (those containing a `.`) before
/// reaching this function.
pub fn query_event_type_alias(short: &str) -> Option<&'static str> {
    Some(match short {
        "glucose" | "std.glucose" | "std.blood_glucose" => "std.blood_glucose",
        "heart_rate" | "std.heart_rate" | "std.heart_rate_resting" => "std.heart_rate_resting",
        "temperature" | "std.temperature" | "std.body_temperature" => "std.body_temperature",
        "medication_taken" | "std.medication_taken" | "std.medication_dose" => {
            "std.medication_dose"
        }
        "symptom" | "std.symptom" => "std.symptom",
        _ => return None,
    })
}

/// Build an `EventInput` for the given kind, stamped at the supplied epoch
/// ms. Returns `Err` for invalid units / out-of-range enum ordinals.
pub fn build_event_input(kind: &LogKind, timestamp_ms: i64) -> Result<pb::EventInput> {
    let event_type = canonical_event_type(kind).to_string();
    let channels = match kind {
        LogKind::Glucose { value, unit } => {
            // Storage canonical unit is mmol/L. Accept mg/dL and convert.
            let mmol_l = match unit.as_deref() {
                None | Some("mmol/L") | Some("mmol/l") => *value,
                Some("mg/dL") | Some("mg/dl") => *value / 18.0182,
                Some(other) => {
                    return Err(anyhow!(
                        "unsupported glucose unit {other:?}; use mmol/L or mg/dL"
                    ));
                }
            };
            vec![real_channel("value", mmol_l)]
        }
        LogKind::HeartRate { bpm } => vec![real_channel("value", *bpm)],
        LogKind::Temperature { value, unit } => {
            // Storage canonical unit is C. Accept F and convert.
            let celsius = match unit.as_deref() {
                None | Some("C") | Some("c") | Some("celsius") => *value,
                Some("F") | Some("f") | Some("fahrenheit") => (*value - 32.0) * 5.0 / 9.0,
                Some(other) => {
                    return Err(anyhow!(
                        "unsupported temperature unit {other:?}; use C or F"
                    ));
                }
            };
            vec![real_channel("value", celsius)]
        }
        LogKind::MedicationTaken {
            name,
            dose,
            dose_unit,
            status,
        } => {
            let mut out = vec![text_channel("name", name)];
            if let Some(d) = dose {
                out.push(real_channel("dose", *d));
            }
            if let Some(unit) = dose_unit {
                let ord = enum_ord_or_err(
                    "dose_unit",
                    unit,
                    &["mg", "mcg", "g", "ml", "units", "tablets", "puffs", "drops"],
                )?;
                out.push(enum_channel("dose_unit", ord));
            }
            // Default status is "taken".
            let status_str = status.as_deref().unwrap_or("taken");
            let ord = enum_ord_or_err(
                "status",
                status_str,
                &["taken", "skipped", "late", "refused"],
            )?;
            out.push(enum_channel("status", ord));
            out
        }
        LogKind::Symptom {
            name,
            severity,
            location,
        } => {
            let mut out = vec![text_channel("name", name)];
            if let Some(s) = severity {
                out.push(int_channel("severity", *s));
            }
            if let Some(loc) = location {
                out.push(text_channel("location", loc));
            }
            out
        }
    };

    Ok(pb::EventInput {
        timestamp_ms,
        event_type,
        channels,
        ..Default::default()
    })
}

// ---- channel-value builders -----------------------------------------------

fn real_channel(path: &str, value: f64) -> pb::ChannelValue {
    pb::ChannelValue {
        channel_path: path.into(),
        value: Some(pb::channel_value::Value::RealValue(value)),
        ..Default::default()
    }
}

fn int_channel(path: &str, value: i64) -> pb::ChannelValue {
    pb::ChannelValue {
        channel_path: path.into(),
        value: Some(pb::channel_value::Value::IntValue(value)),
        ..Default::default()
    }
}

fn text_channel(path: &str, value: &str) -> pb::ChannelValue {
    pb::ChannelValue {
        channel_path: path.into(),
        value: Some(pb::channel_value::Value::TextValue(value.to_string())),
        ..Default::default()
    }
}

fn enum_channel(path: &str, ordinal: i32) -> pb::ChannelValue {
    pb::ChannelValue {
        channel_path: path.into(),
        value: Some(pb::channel_value::Value::EnumOrdinal(ordinal)),
        ..Default::default()
    }
}

fn enum_ord_or_err(channel: &str, value: &str, allowed: &[&str]) -> Result<i32> {
    allowed
        .iter()
        .position(|v| *v == value)
        .map(|i| i as i32)
        .ok_or_else(|| {
            anyhow!(
                "{channel}: {value:?} not in allowed enum {:?}",
                allowed
            )
        })
}

/// Render a `pb::ChannelValue` for the query-results table. Strings stay
/// as-is; reals get a sane fixed-precision; enum ordinals print the index
/// (resolution to label is server-side via `Registry.ResolveChannel` and is
/// deferred per `../STATUS.md`).
pub fn render_channel_value(cv: &pb::ChannelValue) -> String {
    use pb::channel_value::Value;
    match &cv.value {
        Some(Value::RealValue(v)) => format!("{}={v}", cv.channel_path),
        Some(Value::IntValue(v)) => format!("{}={v}", cv.channel_path),
        Some(Value::BoolValue(v)) => format!("{}={v}", cv.channel_path),
        Some(Value::TextValue(v)) => format!("{}={v:?}", cv.channel_path),
        Some(Value::EnumOrdinal(v)) => format!("{}=[#{v}]", cv.channel_path),
        None => format!("{}=<unset>", cv.channel_path),
    }
}
